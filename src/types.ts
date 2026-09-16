// 与 Rust 侧 serde 结构一一对应。字段名/枚举 tag 改动必须两侧同步。

export type Market = "ashare" | "indexcn" | "hk" | "us";

export interface Change {
  amount: number | null;
  percent: number | null;
}

export interface Quote {
  code: string;
  name: string;
  raw_symbol: string;
  market: Market;
  price: number | null;
  prev_close: number | null;
  open: number | null;
  high: number | null;
  low: number | null;
  change: Change;
  price_decimals: number | null;
  raw_timestamp: string;
  quote_time: string | null;
  /** 当日累计成交额（元）。腾讯报文 [38] 万元×10000。null = 该报文不带 */
  amount: number | null;
}

export interface MinutePoint {
  hhmm: number;
  grid_index: number;
  price: number;
  avg: number | null;
  /** true 时前端 moveTo 断线。前端不需要知道任何市场的午休时间 */
  break_before: boolean;
}

export interface MinuteSeries {
  code: string;
  market: Market;
  /** 该市场栅格长度，X 轴映射的分母。前端不自行推算 */
  grid_len: number;
  prev_close: number | null;
  points: MinutePoint[];
}

export type DataState =
  | { kind: "pending" }
  | { kind: "fresh" }
  | { kind: "stale"; last_ok_at: string; from_report_time: boolean }
  | { kind: "error"; message: string };

export interface RowSnapshot {
  code: string;
  /** null = 该标的无已知东财 secid（不猜 105/106/107 前缀） */
  em_secid: string | null;
  market: Market;
  is_index: boolean;
  /**
   * 是否隐藏。**快照里保留隐藏行，过滤只发生在悬浮窗的渲染层** ——
   * 设置页要用它显示名称与价格，提醒判定也要照常。
   */
  hidden: boolean;
  name: string | null;
  quote: Quote | null;
  minute: MinuteSeries | null;
  state: DataState;
  /** false 时不画曲线。严禁用单点/昨收线/插值补假曲线 */
  minute_available: boolean;
  /** 当日资金流（v0.3.0）。null = 无 secid / 该标的不供资金流 / 本轮未取到 */
  fund_flow: FundFlow | null;
}

/** 单只标的当日资金流（元，净流入口径）。主力=超大+大，散户=小单 */
export interface FundFlow {
  main_net: number;
  retail_net: number;
  medium_net: number;
  extra_large_net: number;
  large_net: number;
}

/** 悬停资金流提示窗（flowtip）的内容载荷 */
export interface FlowTipPayload {
  main_net: number;
  retail_net: number;
  market: Market | null;
}

export interface Snapshot {
  rows: RowSnapshot[];
  updated_at: string;
  quote_ok: boolean;
}

// ---------- Sprint 2A：配置体系 ----------

export interface WatchItem {
  /** 唯一身份键（`sh000001` 形式，市场前缀内嵌）。**不是**东财裸码 */
  tencent_code: string;
  /** null = 无已知东财 secid（不猜 105/106/107 前缀） */
  em_secid: string | null;
  market: Market;
  is_index: boolean;
  /**
   * 是否在悬浮窗中隐藏。**隐藏 ≠ 删除。**
   *
   * 只影响悬浮窗显示；后台照常取数、照常判定提醒。所以隐藏的标的仍会
   * 出现在设置页（带标记，可取消），取消后立刻有数据不必等下一轮。
   */
  hidden: boolean;
}

export interface AppearanceConfig {
  scale: number;
  curve_width: number;
  font_size: number;
  show_code: boolean;
  show_market: boolean;
  /** 0 时文字与曲线仍须清晰可见（三组不透明度落在互不嵌套的 DOM 层） */
  opacity_background: number;
  opacity_text: number;
  opacity_curve: number;
  /** 界面主题。`null`/缺失 = 深色（与加这个字段之前的行为一致）。悬浮窗不做主题 */
  theme: "dark" | "light" | null;
  /** 悬浮窗行内是否显示主力/散户资金流（v0.3.0）。默认开 */
  show_fund_flow: boolean;
}

/**
 * 数据刷新间隔（秒）。
 *
 * 区间由后端 `sanitize()` 强制钳制，UI 也只提供区间内的选项。
 * 下限不同的原因：报价走批量接口（一次拿全部自选股，与数量无关），
 * 而分时是**每个标的一个请求** —— 间隔要乘以自选股数量才是真实请求速率，
 * 所以它的下限更高。
 */
export interface RefreshConfig {
  /** 报价（价格/涨跌幅）刷新间隔。合法区间 2~60 */
  quote_secs: number;
  /** 分时曲线刷新间隔。合法区间 3~300 */
  minute_secs: number;
}

/** 与 Rust 侧 QUOTE_SECS_MIN/MAX、MINUTE_SECS_MIN/MAX 保持一致 */
export const REFRESH_LIMITS = {
  quote: { min: 2, max: 60 },
  minute: { min: 3, max: 300 },
} as const;

export interface WindowConfig {
  x: number | null;
  y: number | null;
  always_on_top: boolean;
  /** 锁定 = 不可拖动 + 点击穿透。解锁的唯一通道是托盘菜单 */
  locked: boolean;
  /**
   * 悬浮窗形态：`true` = 收起为球（单只轮播），`false` = 展开列表。
   *
   * `null`（旧配置未写过这个字段）**等同 true**，即球态 —— 球是主形态。
   * 读取一律用 `isCollapsed()`（lib/ballRotate 之外的地方直接写
   * `?? true` 也可以），**不要写 `?? false`**，那会让老用户升级后直接展开。
   */
  collapsed: boolean | null;
}

/** 日K 均线配置（v0.3.0）：升序去重、区间 2~250、≤8 条（后端 sanitize 强制） */
export interface ChartConfig {
  ma_periods: number[];
}

export interface AppConfig {
  schemaVersion: number;
  watchlist: WatchItem[];
  appearance: AppearanceConfig;
  window: WindowConfig;
  alerts: AlertConfig;
  refresh: RefreshConfig;
  charts: ChartConfig;
}

export interface SearchItem {
  tencent_code: string;
  /** 东财裸码，仅用于展示（不唯一，绝不作身份键） */
  raw_code: string;
  name: string;
  market_label: string;
  em_secid: string | null;
  is_index: boolean;
  already_added: boolean;
}

/** 限流与"无匹配结果"必须分开呈现，否则用户会以为标的不存在 */
export type SearchResponse =
  | { kind: "ok"; items: SearchItem[] }
  | { kind: "unavailable"; message: string };

/**
 * 直接输代码的解析结果。
 *
 * 四态**必须分开呈现**：
 * - `invalid` 是输入本身的问题（打了中文/位数不对）
 * - `not_found` 是格式对但查无此码
 * - `unavailable` 是网断了，代码可能完全正确
 *
 * 把 unavailable 显示成"代码不存在"会让用户以为自己记错了代码，去改一个本来对的输入。
 */
export type ResolveResponse =
  | { kind: "ok"; items: SearchItem[] }
  | { kind: "invalid"; message: string }
  | { kind: "not_found"; message: string }
  | { kind: "unavailable"; message: string };

// ───────────── 大图窗（方案 A）─────────────

/** 一根日K。字段顺序按 OHLC 惯例；后端已把报文的「开收高低」换位。 */
export interface KlineBar {
  date: string;
  open: number;
  high: number;
  low: number;
  close: number;
  volume: number;
}

/** 5 日分时里的一分钟。volume 已由后端从累计值差分得出。 */
export interface DayMinutePoint {
  hhmm: number;
  price: number;
  volume: number;
}

/** 5 日分时的一个交易日。 */
export interface DaySeries {
  date: string;
  prev_close: number | null;
  points: DayMinutePoint[];
}

/** 一条均线的周期与逐根值（前 period-1 根为 null） */
export interface MaLine {
  period: number;
  values: (number | null)[];
}

/** 大图窗一屏数据。后端按 kind 分派，前端据此选渲染器。 */
export type ChartPayload =
  | { kind: "intraday"; series: MinuteSeries }
  | { kind: "fiveDay"; days: DaySeries[]; grid_len: number }
  | { kind: "dailyK"; bars: KlineBar[]; mas: MaLine[] };

export type ChartPeriod = "intraday" | "5day" | "dayk";

// ───────────── 大盘面板 ─────────────

/** 一档资金流向（单位：元，不缩放） */
export interface FlowTier {
  inflow: number;
  outflow: number;
  net: number;
}

/** 五档资金流向。主力 = extra_large.net + large.net（东财口径，实测等于 total.net） */
export interface MoneyFlow {
  total: FlowTier;
  extra_large: FlowTier;
  large: FlowTier;
  medium: FlowTier;
  small: FlowTier;
}

export interface Breadth {
  up: number;
  down: number;
  flat: number;
}

export interface MarketStats {
  name: string;
  breadth: Breadth;
  flow: MoneyFlow;
}

export interface MarketRow {
  label: string;
  secid: string;
  /** null 时看 error —— 某个市场失败不该让整个面板空白 */
  stats: MarketStats | null;
  error: string | null;
  /**
   * 该市场数据的取得时刻 `HH:MM:SS`。
   *
   * **必须显示出来。** 后端是轮转取数的（每轮只取一个数据点，因为东财限流
   * 只允许约每分钟一个请求），所以各市场的新鲜度天然不同。
   * 不标出来用户会以为所有数字都是同一时刻的 —— 那比数据旧更误导。
   */
  fetched_at: string | null;
}

export interface BreadthTotal {
  up: number;
  down: number;
  flat: number;
  /**
   * 参与合计的市场数（0~3）。**小于 3 时必须在界面上标注「数据不全」** ——
   * 否则用户会把「沪+深」的家数当成全 A 股的，少算北交所约 338 只。
   */
  markets_counted: number;
}

/** 大盘面板一屏数据 */
export interface MarketOverview {
  indices: IndexQuote[];
  indices_error: string | null;
  /** 指数数据的取得时刻。同 MarketRow.fetched_at 的理由 */
  indices_at: string | null;
  markets: MarketRow[];
  breadth_total: BreadthTotal;
  /** 主力资金净流入合计（元） */
  main_force_net: number;
  updated_at: string;
}

export interface IndexQuote {
  code: string;
  name: string;
  price: number;
  change_percent: number;
  change_amount: number;
  amount: number | null;
}

// ───────────── 牛熊提醒 ─────────────

export type AlertSide = "bull" | "bear";
export type AlertTrigger = "percent" | "targetPrice";

/** 一次提醒触发。后端 emit `alert` 事件时带一个数组（同周期可能多只同时越界） */
export interface AlertEvent {
  code: string;
  side: AlertSide;
  trigger: AlertTrigger;
  price: number;
  percent: number | null;
  threshold: number;
}

/** 单只标的的提醒规则。四项都可独立启用；null = 不启用 */
export interface AlertRule {
  up_percent: number | null;
  down_percent: number | null;
  bull_price: number | null;
  bear_price: number | null;
  /**
   * 基准价。**不参与判定**，只是算目标价的输入：
   * 填基准价 + 涨跌幅 → 自动算出上下目标价。
   *
   * 旧配置没这个字段时后端补 `None`（`Option<f64>` + serde default），
   * 所以不需要 schema 迁移 —— 见 tests/config_migrate.rs 的
   * `v5_rule_without_base_price_loads_as_none`。
   */
  base_price: number | null;
}

export interface AlertConfig {
  enabled: boolean;
  bull_sound: boolean;
  bear_sound: boolean;
  /**
   * 上涨音色。内置音色 id 或 `custom:文件名`。
   *
   * ⚠️ **这三个字段必须在这里声明**（而不是只存在于 Rust 侧）。
   * `AlertConfig` 在 Rust 上带 `#[serde(default)]`：前端调
   * `update_alert_settings` 时若传的对象缺这几项，serde 会用默认值填上 ——
   * 表现是"用户改了冷却期，音色悄悄被重置回清铃"。
   * 前端类型完整 → `patch()` 基于完整对象展开 → 不会漏字段。
   */
  bull_tone: string;
  bear_tone: string;
  /** 提示音音量 0~1 */
  sound_volume: number;
  card_opacity: number;
  cooldown_secs: number;
  default_up_percent: number | null;
  default_down_percent: number | null;
  rules: Record<string, AlertRule>;
}
