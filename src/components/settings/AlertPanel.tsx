import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { AlertConfig, AlertRule, AppConfig, Snapshot } from "../../types";
import { applyBasePrice, resolvePercents } from "../../lib/alertCalc";
import Slider from "./Slider";
import NumberInput from "./NumberInput";
import { displayCode } from "../../lib/format";
import { demoAlertPopup, listAlertSounds } from "../../lib/ipc";
import SoundRow from "./SoundRow";

interface Props {
  config: AppConfig | null;
  onChange: (next: AppConfig) => void;
}

const EMPTY_RULE: AlertRule = {
  up_percent: null,
  down_percent: null,
  bull_price: null,
  bear_price: null,
  base_price: null,
};

// 原先这里有 toNum / numStr 两个辅助函数（文本 ↔ 数值转换）。
// 全部数字输入改用 NumberInput 组件后，这两件事收敛进了组件内部，
// 这里不再需要 —— 留着会变成两处各自演化的转换规则。

/**
 * 到价提醒设置。功能集包含：
 * 全局阈值 + 逐股目标价 + 按现价一键生成上下目标 + 牛叫熊叫分别开关 + 一键总关。
 */
export default function AlertPanel({ config, onChange }: Props) {
  const alerts = config?.alerts;
  const [prices, setPrices] = useState<Record<string, number>>({});
  const [names, setNames] = useState<Record<string, string>>({});
  /** 已导入的自定义音频文件名，供两个 SoundRow 共用 */
  const [customs, setCustoms] = useState<string[]>([]);

  const reloadCustoms = useCallback(() => {
    listAlertSounds()
      .then(setCustoms)
      .catch(() => setCustoms([]));
  }, []);

  useEffect(() => reloadCustoms(), [reloadCustoms]);
  // 目标价输入用本地态：边打字边写配置会让光标跳动
  const [draft, setDraft] = useState<Record<string, AlertRule>>({});

  // 目标价模式要实时显示最新价（参考项目也是这么做的，方便按现价定目标）
  useEffect(() => {
    const pull = () => {
      invoke<Snapshot>("get_snapshot")
        .then((snap) => {
          const p: Record<string, number> = {};
          const n: Record<string, string> = {};
          for (const r of snap.rows) {
            if (r.quote?.price != null) p[r.code] = r.quote.price;
            if (r.name) n[r.code] = r.name;
          }
          setPrices(p);
          setNames(n);
        })
        .catch(() => void 0);
    };
    pull();
    const t = window.setInterval(pull, 3000);
    return () => window.clearInterval(t);
  }, []);

  /**
   * 从配置回填 draft。
   *
   * **依赖必须是「标的集合」而不是 `alerts.rules` 对象本身。**
   * 原写法 `}, [alerts?.rules])` 有个自我回冲的 bug：每次 `saveRule` 后端都返回
   * 一个**新的** AppConfig 对象，`alerts.rules` 引用随之改变 → effect 重跑 →
   * 用服务端数据**整体覆盖** draft。后果是用户在 A 股票框里打了字还没失焦，
   * 一旦 B 股票那边触发保存，A 的未保存输入就被抹掉。
   *
   * 现在只在标的集合变化时同步（新增/删除规则），且**逐码合并而非整体替换**：
   * draft 里已有的码保留用户正在编辑的值，只补进 draft 里没有的码。
   * 设置窗是提醒规则的唯一写入方，所以不必担心漏掉外部改动。
   */
  const ruleCodesKey = Object.keys(alerts?.rules ?? {})
    .sort()
    .join(",");

  useEffect(() => {
    const serverRules = alerts?.rules ?? {};
    setDraft((prev) => {
      const next: Record<string, AlertRule> = {};
      for (const code of Object.keys(serverRules)) {
        // 已在编辑的保留 draft，未编辑过的从服务端取
        next[code] = prev[code] ?? serverRules[code];
      }
      return next;
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ruleCodesKey]);

  if (!config || !alerts) {
    return <div className="panel">配置加载中…</div>;
  }

  const patch = (p: Partial<AlertConfig>) => {
    const settings: AlertConfig = { ...alerts, ...p };
    invoke<AppConfig>("update_alert_settings", { settings })
      .then(onChange)
      .catch((e) => console.error("更新提醒设置失败:", e));
  };

  const saveRule = (code: string, rule: AlertRule) => {
    invoke<AppConfig>("set_alert_rule", { code, rule })
      .then(onChange)
      .catch((e) => console.error("保存提醒规则失败:", e));
  };

  const ruleOf = (code: string): AlertRule => draft[code] ?? EMPTY_RULE;

  const editDraft = (code: string, p: Partial<AlertRule>) => {
    setDraft((prev) => ({ ...prev, [code]: { ...ruleOf(code), ...p } }));
  };

  /**
   * 基准价 + 涨跌幅 → 自动算出上下目标价。
   *
   * 只在**基准价或涨跌幅失焦时**触发（不在目标价失焦时触发），这样：
   * 手动改过的目标价会一直保留，直到用户重新动基准价或涨跌幅为止。
   * 若边打字边算，用户想手填目标价会被不断覆盖掉。
   *
   * 缺基准价、或缺对应方向的涨跌幅时，该方向的目标价**原样不动** ——
   * 不清成 null（用户可能是手填的），也不猜一个值。
   */
  const recalcFromBase = (code: string) => {
    const next = applyBasePrice(ruleOf(code));
    editDraft(code, next);
    saveRule(code, next);
  };

  /**
   * 按现价一键生成止盈价/止跌价。
   *
   * **语义：现价作为基准价，再用涨跌幅推出两个目标价。**
   * 涨跌幅取值顺序：逐股 `up_percent`/`down_percent` → 全局默认阈值
   * `default_up_percent`/`default_down_percent` → 兜底 3。
   *
   * 2026-08-30 修：原来是硬编码 `p*1.03` / `p*0.97`，**与用户填的涨跌幅无关**。
   * 所以逐股填了 5% 再点这个按钮，出来的仍是 ±3% 的数 ——
   * 用户看到的现象是"点击按现价生成，没变化"（它算的东西跟预期不是一回事）。
   * 现在改为走 `applyBasePrice`，与「基准价/涨跌幅失焦后自动重算」完全同一条路径，
   * 两个入口不会再算出不同结果。
   *
   * 为什么缺值时回退**全局默认阈值**而不是另定一对数字：
   * 全局阈值就是用户已经在用的"默认"，两处语义一致才不会打架。
   * 全局也没填（null）时才用 3 兜底 —— 保持与改动前的行为一致。
   */
  const genFromPrice = (code: string) => {
    const p = prices[code];
    if (p == null) return;
    const cur = ruleOf(code);
    const { up, down } = resolvePercents(cur, {
      up: alerts?.default_up_percent ?? null,
      down: alerts?.default_down_percent ?? null,
    });
    // 先把基准价与生效涨跌幅落进规则，再交给纯函数推目标价。
    // 涨跌幅一并写回：否则面板上会显示"目标价按 5% 算出来的、涨跌幅栏却是空的"，
    // 用户无法看出这两个数的由来。
    const rule = applyBasePrice({
      ...cur,
      base_price: p,
      up_percent: up,
      down_percent: down,
    });
    editDraft(code, rule);
    saveRule(code, rule);
  };

  return (
    <div className="panel alert-panel">
      <section className="card">
        <label className="row-switch">
          <input
            type="checkbox"
            checked={alerts.enabled}
            onChange={(e) => patch({ enabled: e.target.checked })}
          />
          <span>启动提醒</span>
        </label>
        <p className="hint">
          越过上涨阈值或止盈价时提醒，跌破下跌阈值或止跌价时提醒。
          每次越界只提醒一次，价格回到阈值内侧 5% 后才重新布防。非交易时段不提醒。
        </p>
      </section>

      <fieldset className="card" disabled={!alerts.enabled}>
        <legend>全局默认阈值</legend>
        <div className="pair">
          <label>
            <span>上涨阈值</span>
            <span className="inline-input">
              +
              <NumberInput
                value={alerts.default_up_percent}
                step={0.1}
                min={0}
                decimals={1}
                onChange={(default_up_percent) => patch({ default_up_percent })}
              />
              %
            </span>
          </label>
          <label>
            <span>下跌阈值</span>
            <span className="inline-input">
              −
              <NumberInput
                value={alerts.default_down_percent}
                step={0.1}
                min={0}
                decimals={1}
                onChange={(default_down_percent) =>
                  patch({ default_down_percent })
                }
              />
              %
            </span>
          </label>
        </div>
        <p className="hint">留空表示不启用。逐股设置会覆盖这里的默认值。</p>
      </fieldset>

      <fieldset className="card" disabled={!alerts.enabled}>
        <legend>提示音与外观</legend>
        {/* 上涨 / 下跌各一组：开关 + 音色 + 试听。
            原来这里写的是「牛叫（上涨提示音）」，但实际播的一直是 WebAudio
            合成的双音 —— 文案在骗人。现在文案与实现一致，且音色可选。 */}
        <SoundRow
          label="上涨提示音"
          enabled={alerts.bull_sound}
          tone={alerts.bull_tone}
          volume={alerts.sound_volume}
          customs={customs}
          up
          onToggle={(v) => patch({ bull_sound: v })}
          onTone={(v) => patch({ bull_tone: v })}
          onImported={reloadCustoms}
        />
        <SoundRow
          label="下跌提示音"
          enabled={alerts.bear_sound}
          tone={alerts.bear_tone}
          volume={alerts.sound_volume}
          customs={customs}
          up={false}
          onToggle={(v) => patch({ bear_sound: v })}
          onTone={(v) => patch({ bear_tone: v })}
          onImported={reloadCustoms}
        />

        <Slider
          label="提示音音量"
          value={alerts.sound_volume}
          min={0}
          max={1}
          step={0.05}
          format={(v) => `${Math.round(v * 100)}%`}
          onInput={(sound_volume) => patch({ sound_volume })}
          rowClass="row-slider"
        />
        <Slider
          label="卡片不透明度"
          value={alerts.card_opacity}
          min={0.2}
          max={1}
          step={0.01}
          format={(v) => `${Math.round(v * 100)}%`}
          onInput={(card_opacity) => patch({ card_opacity })}
          rowClass="row-slider"
        />
        <Slider
          label="冷却期"
          value={alerts.cooldown_secs}
          min={5}
          max={1800}
          step={5}
          format={(v) => `${v}s`}
          onInput={(cooldown_secs) => patch({ cooldown_secs })}
          rowClass="row-slider"
        />
        <p className="hint">
          冷却期内同一条规则不重复提醒，避免在阈值边缘反复响。
        </p>

        <div className="demo-row">
          <button
            type="button"
            className="demo-popup-btn"
            onClick={() => {
              void demoAlertPopup().catch((e) =>
                console.error("预览弹窗失败:", e)
              );
            }}
            title="在屏幕中心弹出一次提醒，效果与真实到价时一致"
          >
            预览弹窗效果
          </button>
          <span className="hint-text">
            到价时会在屏幕中心弹窗提示，几秒后自动消失（点击可立即关闭）。
          </span>
        </div>
      </fieldset>

      <fieldset className="card" disabled={!alerts.enabled}>
        <legend>逐股规则</legend>
        {config.watchlist.length === 0 && (
          <p className="hint">自选股为空，先去「自选股」页添加。</p>
        )}
        {config.watchlist.map((w) => {
          const code = w.tencent_code;
          const r = ruleOf(code);
          const px = prices[code];
          return (
            <div className="alert-rule" key={code}>
              <div className="ar-head">
                <span className="ar-name">
                  {names[code] ?? displayCode(code)}
                </span>
                <span className="ar-code">{displayCode(code)}</span>
                <span className="ar-price">
                  {px != null ? px.toFixed(2) : "—"}
                </span>
                <button
                  className="ar-gen"
                  disabled={px == null}
                  onClick={() => genFromPrice(code)}
                  title="按现价 ±3% 生成止盈价与止跌价"
                >
                  按现价生成
                </button>
              </div>
              <div className="ar-inputs">
                <label>
                  {/* 放第一列：它是右边两个目标价的计算来源，视觉顺序跟着数据流走 */}
                  <span>基准价</span>
                  <NumberInput
                    value={r.base_price}
                    step={0.01}
                    min={0}
                    title="填基准价 + 涨跌幅，失焦后自动算出止盈价与止跌价。聚焦后可用滚轮微调"
                    onChange={(base_price) => editDraft(code, { base_price })}
                    onCommit={() => recalcFromBase(code)}
                  />
                </label>
                <label>
                  <span>止盈价</span>
                  <NumberInput
                    value={r.bull_price}
                    step={0.01}
                    min={0}
                    onChange={(bull_price) => editDraft(code, { bull_price })}
                    onCommit={() => saveRule(code, ruleOf(code))}
                  />
                </label>
                <label>
                  <span>止跌价</span>
                  <NumberInput
                    value={r.bear_price}
                    step={0.01}
                    min={0}
                    onChange={(bear_price) => editDraft(code, { bear_price })}
                    onCommit={() => saveRule(code, ruleOf(code))}
                  />
                </label>
                <label>
                  <span>涨幅 %</span>
                  <NumberInput
                    value={r.up_percent}
                    step={0.1}
                    min={0}
                    decimals={1}
                    placeholder="用默认"
                    onChange={(up_percent) => editDraft(code, { up_percent })}
                    // 走 recalcFromBase 而不是 saveRule：填了基准价时这里一改
                    // 就该重算止盈价。没填基准价时它退化成普通保存。
                    onCommit={() => recalcFromBase(code)}
                  />
                </label>
                <label>
                  <span>跌幅 %</span>
                  <NumberInput
                    value={r.down_percent}
                    step={0.1}
                    min={0}
                    decimals={1}
                    placeholder="用默认"
                    onChange={(down_percent) => editDraft(code, { down_percent })}
                    onCommit={() => recalcFromBase(code)}
                  />
                </label>
              </div>
            </div>
          );
        })}
      </fieldset>
    </div>
  );
}
