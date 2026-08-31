import { useEffect, useRef, useState } from "react";
import type { SearchItem } from "../../types";
import { searchSymbols } from "../../lib/ipc";

interface Props {
  onPick: (item: SearchItem) => void;
  /** 已达上限时禁用添加（仍允许搜索，便于用户确认标的是否存在） */
  limitReached: boolean;
}

/** 输入防抖（D9）。太短会让每个字符都打一次接口，容易触发限流 */
const DEBOUNCE_MS = 250;

type State =
  | { kind: "idle" }
  | { kind: "loading" }
  | { kind: "ok"; items: SearchItem[] }
  /** 限流/两源皆失败。**必须与"无匹配结果"分开呈现** */
  | { kind: "unavailable"; message: string }
  | { kind: "error"; message: string };

export default function SearchBox({ onPick, limitReached }: Props) {
  const [query, setQuery] = useState("");
  const [state, setState] = useState<State>({ kind: "idle" });
  /** 请求竞态保护（D9）：只接受最新一次请求的响应 */
  const seqRef = useRef(0);

  useEffect(() => {
    const q = query.trim();
    if (q === "") {
      setState({ kind: "idle" });
      return;
    }

    const seq = ++seqRef.current;
    setState({ kind: "loading" });
    const timer = window.setTimeout(() => {
      searchSymbols(q)
        .then((resp) => {
          // 先发的慢响应不得覆盖后发的快响应
          if (seq !== seqRef.current) return;
          if (resp.kind === "unavailable") {
            setState({ kind: "unavailable", message: resp.message });
          } else {
            setState({ kind: "ok", items: resp.items });
          }
        })
        .catch((e) => {
          if (seq !== seqRef.current) return;
          setState({ kind: "error", message: String(e) });
        });
    }, DEBOUNCE_MS);

    return () => window.clearTimeout(timer);
  }, [query]);

  return (
    <div className="search-box">
      <label className="field-label" htmlFor="symbol-search">
        搜索标的
      </label>
      <input
        id="symbol-search"
        className="text-input"
        type="text"
        value={query}
        placeholder="名称 / 代码 / 拼音，如 茅台、600519、GZMT"
        onChange={(e) => setQuery(e.target.value)}
        autoComplete="off"
        // Sprint 3 会做全局快捷键：届时若在此处加键盘处理，
        // 需与全局热键的作用域隔离（输入框内不应被全局热键抢走按键）
      />

      {state.kind === "loading" && <div className="search-hint">搜索中…</div>}

      {state.kind === "unavailable" && (
        // 限流文案与"无匹配结果"严格区分：不能让用户以为标的不存在
        <div className="search-hint search-warn" role="status">
          {state.message}
        </div>
      )}

      {state.kind === "error" && (
        <div className="search-hint search-warn" role="status">
          搜索失败：{state.message}
        </div>
      )}

      {state.kind === "ok" && state.items.length === 0 && (
        <div className="search-hint" role="status">
          无匹配结果
        </div>
      )}

      {state.kind === "ok" && state.items.length > 0 && (
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
                  onClick={() => onPick(it)}
                >
                  <span className="si-name">{it.name}</span>
                  <span className="si-code">{it.raw_code}</span>
                  <span className="si-market">{it.market_label}</span>
                  {disabled && <span className="si-added">{why}</span>}
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
