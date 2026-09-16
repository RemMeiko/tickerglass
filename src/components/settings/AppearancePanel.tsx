import {
  REFRESH_LIMITS,
  type AppConfig,
  type AppearanceConfig,
  type RefreshConfig,
} from "../../types";
import { updateAppearance, updateRefresh } from "../../lib/ipc";
import Slider from "./Slider";

interface Props {
  config: AppConfig;
}

/**
 * 外观面板。**没有"应用"按钮** —— 标准 12 要求拖动滑块 ≤200ms 立即生效。
 *
 * 用 `onInput`（不是 `onChange`）：React 的 `onChange` 对 range input 已等价于
 * input 事件，但显式写 `onInput` 让"拖动过程中就更新"这个意图不依赖框架细节。
 * 每次变更立刻发 IPC，后端 emit 广播 → 悬浮窗即时预览；写盘在后端侧完成。
 */
export default function AppearancePanel({ config }: Props) {
  const a = config.appearance;
  // 旧配置可能没有 refresh 段（后端 serde default 会补，但首帧配置来自
  // 事件推送时理论上可能还没补上），给个兜底避免读 undefined
  const r = config.refresh ?? { quote_secs: 3, minute_secs: 5 };

  function patch(next: Partial<AppearanceConfig>) {
    void updateAppearance({ ...a, ...next }).catch((e) =>
      console.error("更新外观失败:", e)
    );
  }

  /**
   * 刷新间隔走**独立的 IPC 命令**，不复用 update_appearance。
   *
   * 理由：它不属于 appearance 段。混进去会让 `update_appearance`
   * 的语义变成"更新外观和一些别的东西"，而且两段的钳制规则不同
   * （外观钳的是缩放/透明度，这里钳的是请求速率上下限）。
   */
  function patchRefresh(next: Partial<RefreshConfig>) {
    void updateRefresh({ ...r, ...next }).catch((e) =>
      console.error("更新刷新间隔失败:", e)
    );
  }

  return (
    <div className="panel">
      <Slider
        label="整体缩放"
        value={a.scale}
        min={0.65}
        max={1.6}
        step={0.05}
        format={(v) => `${Math.round(v * 100)}%`}
        onInput={(scale) => patch({ scale })}
      />

      <Slider
        label="曲线宽度"
        value={a.curve_width}
        min={0.5}
        max={4}
        step={0.1}
        format={(v) => `${v.toFixed(1)} px`}
        onInput={(curve_width) => patch({ curve_width })}
      />

      <Slider
        label="字号"
        value={a.font_size}
        min={8}
        max={24}
        step={1}
        format={(v) => `${v} px`}
        onInput={(font_size) => patch({ font_size })}
      />

      {/* 主题切换已移到设置窗 Tab 栏右端的日/月图标按钮（ThemeToggle.tsx）。
          这里原本有一组「夜间 / 白天」文字分段控件，删除理由：
          主题是全局开关，藏在外观页里用户得先找到这一页；
          且两处入口并存必然出现"改了一处另一处没同步"的观感问题。 */}

      <div className="field-row">
        <label className="check">
          <input
            type="checkbox"
            checked={a.show_code}
            onChange={(e) => patch({ show_code: e.target.checked })}
          />
          显示证券代码
        </label>
        <label className="check">
          <input
            type="checkbox"
            checked={a.show_market}
            onChange={(e) => patch({ show_market: e.target.checked })}
          />
          显示市场标识
        </label>
        <label className="check">
          <input
            type="checkbox"
            // 旧配置无此字段时按默认开处理（后端 serde default 同值）
            checked={a.show_fund_flow ?? true}
            onChange={(e) => patch({ show_fund_flow: e.target.checked })}
          />
          悬停显示主力/散户资金
        </label>
      </div>

      <div className="group-title">不透明度（三组互相独立）</div>

      <Slider
        label="背景板"
        value={a.opacity_background}
        min={0}
        max={1}
        step={0.01}
        format={pct}
        onInput={(opacity_background) => patch({ opacity_background })}
      />
      <Slider
        label="文字"
        value={a.opacity_text}
        min={0}
        max={1}
        step={0.01}
        format={pct}
        onInput={(opacity_text) => patch({ opacity_text })}
      />
      <Slider
        label="曲线"
        value={a.opacity_curve}
        min={0}
        max={1}
        step={0.01}
        format={pct}
        onInput={(opacity_curve) => patch({ opacity_curve })}
      />

      <p className="hint-text">
        背景板调到 0 时文字与曲线仍清晰可见，窗口也仍可拖动。
      </p>

      <div className="group-title">刷新间隔</div>
      <Slider
        label="行情价格"
        value={r.quote_secs}
        min={REFRESH_LIMITS.quote.min}
        max={REFRESH_LIMITS.quote.max}
        step={1}
        format={(v) => `${v} 秒`}
        onInput={(quote_secs) => patchRefresh({ quote_secs })}
      />
      <Slider
        label="分时曲线"
        value={r.minute_secs}
        min={REFRESH_LIMITS.minute.min}
        max={REFRESH_LIMITS.minute.max}
        step={1}
        format={(v) => `${v} 秒`}
        onInput={(minute_secs) => patchRefresh({ minute_secs })}
      />
      <p className="hint-text">
        改完立即生效，无需重启。价格走批量接口，间隔与自选股数量无关；
        分时是每只股票单独请求，间隔太短会被数据源限流（美股另有自动降频保护）。
      </p>
    </div>
  );
}

function pct(v: number): string {
  return `${Math.round(v * 100)}%`;
}

// Slider 已抽到 ./Slider.tsx 供本页与提醒页共用。
// 抽离原因：这个"拖动期间走本地态"的修复原先只做在本页，
// 提醒页那三个滑块是各自写的裸 input，于是那边仍然卡顿。
// 收敛成一个组件后，交互修复对两页同时生效。
