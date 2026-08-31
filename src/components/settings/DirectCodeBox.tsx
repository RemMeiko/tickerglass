import { useRef, useState } from "react";
import type { SearchItem } from "../../types";
import { resolveCode } from "../../lib/ipc";
import { displayCode } from "../../lib/format";

interface Props {
  onPick: (item: SearchItem) => void;
  limitReached: boolean;
}

/**
 * 直接输代码添加（A 股）。
 *
 * 与 `SearchBox` 并存而不是合并的三个理由：
 *
 * 1. **触发方式不同**：搜索是边打边查（防抖 250ms），代码必须**回车才查** ——
 *    打 `600519` 的过程中会经过 `6`/`60`/`600`…，边打边查等于白打 5 次接口，
 *    而且 `000001` 这种前缀本身就是合法码，中间态会弹出误导性结果。
 * 2. **走的接口不同**：这里用报价接口，搜索限流时仍可用。
 * 3. **歧义处理不同**：搜索结果是"匹配度排序"，这里是"同一个码的不同市场"，
 *    必须让用户看清是沪市还是深市再选。
 */
export default function DirectCodeBox({ onPick, limitReached }: Props) {
  const [input, setInput] = useState("");
  const [state, setState] = useState<
    | { kind: "idle" }
    | { kind: "loading" }
    | { kind: "ok"; items: SearchItem[] }
    | { kind: "invalid"; message: string }
    | { kind: "not_found"; message: string }
    | { kind: "unavailable"; message: string }
  >({ kind: "idle" });
  /** 竞态保护：快速连按回车时只认最后一次的响应 */
  const seqRef = useRef(0);

  async function submit() {
    const q = input.trim();
    if (q === "") return;
    const seq = ++seqRef.current;
    setState({ kind: "loading" });
    try {
      const resp = await resolveCode(q);
      if (seq !== seqRef.current) return;
      setState(resp);
    } catch (e) {
      if (seq !== seqRef.current) return;
      // 后端 Err（如 HTTP 客户端构建失败）。归到 unavailable：
      // 这不是"代码不存在"，别让用户去改一个本来正确的输入
      setState({ kind: "unavailable", message: String(e) });
    }
  }

  function handlePick(item: SearchItem) {
    onPick(item);
    // 添加后清空，方便连续输入下一只
    setInput("");
    setState({ kind: "idle" });
    seqRef.current++;
  }

  return (
    <div className="direct-code-box">
      <label className="field-label" htmlFor="direct-code">
        直接输代码
      </label>
      <div className="dc-row">
        <input
          id="direct-code"
          className="text-input"
          type="text"
          value={input}
          placeholder="如 600519、000001、sz300750、430047"
          autoComplete="off"
          onChange={(e) => {
            setInput(e.target.value);
            // 输入变了就把上一次的结果撤掉，避免"结果与输入框不一致"
            if (state.kind !== "idle") setState({ kind: "idle" });
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              void submit();
            }
          }}
        />
        <button
          className="dc-submit"
          disabled={input.trim() === "" || state.kind === "loading"}
          onClick={() => void submit()}
        >
          {state.kind === "loading" ? "查询中…" : "查询"}
        </button>
      </div>

      {state.kind === "idle" && (
        <div className="search-hint">
          支持沪深/科创/创业板/北交所与 ETF。输入后按回车查询
        </div>
      )}

      {state.kind === "invalid" && (
        <div className="search-hint search-warn" role="status">
          {state.message}
        </div>
      )}

      {state.kind === "not_found" && (
        <div className="search-hint search-warn" role="status">
          {state.message}
        </div>
      )}

      {state.kind === "unavailable" && (
        // 与 not_found 分开：网断了不代表代码错了
        <div className="search-hint search-warn" role="status">
          {state.message}
        </div>
      )}

      {state.kind === "ok" && (
        <>
          {state.items.length > 1 && (
            // 歧义码：这句必须有，否则用户不知道为什么同一个码出来两条
            <div className="search-hint">
              该代码在多个市场都存在，请选择要添加的那个
            </div>
          )}
          <ul className="search-results">
            {state.items.map((it) => {
              const disabled = it.already_added || limitReached;
              const why = it.already_added
                ? "已在自选股中"
                : limitReached
                  ? "已达 50 只上限"
                  : "";
              return (
                <li key={it.tencent_code}>
                  <button
                    className="search-item"
                    disabled={disabled}
                    title={why}
                    onClick={() => handlePick(it)}
                  >
                    <span className="si-name">{it.name}</span>
                    {/* 显示裸码而不是内部身份键：与搜索框（用 raw_code）保持一致，
                        也与行情软件的习惯一致。歧义靠右边的市场标签区分。 */}
                    <span className="si-code">
                      {displayCode(it.tencent_code)}
                    </span>
                    <span className="si-market">{it.market_label}</span>
                    {disabled && <span className="si-added">{why}</span>}
                  </button>
                </li>
              );
            })}
          </ul>
        </>
      )}
    </div>
  );
}
