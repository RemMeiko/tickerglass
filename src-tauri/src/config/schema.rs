// 配置的 serde 定义与默认值。**纯层**：不 use reqwest / tauri。
//
// 字段命名：除 `schemaVersion` 外一律 snake_case，与 Sprint 1 既有的
// `RowSnapshot`/`MinuteSeries` 保持一致（前端 types.ts 也是 snake_case）。

use crate::model::WatchItem;
use crate::watchlist::default_watchlist;
use serde::{Deserialize, Serialize};

/// 当前配置结构版本。
///
/// - v1 = Sprint 1 时代的隐式结构（只有自选股概念，无外观/窗口配置）
/// - v2 = Sprint 2A：新增 `appearance` 与 `window`
/// 配置结构版本。**加字段就要升它并补迁移**（见 `config::migrate`）。
///
/// 变更史：
/// - 2 → 3：提醒体系（AlertConfig）
/// - 3 → 4：提示音可选音色与音量（`bull_tone`/`bear_tone`/`sound_volume`）
/// - 4 → 5：自选股可隐藏（`WatchItem.hidden`）
pub const SCHEMA_VERSION: u32 = 5;

/// 缩放下限/上限（spec 标准 10：65% → 160%）。
pub const SCALE_MIN: f64 = 0.65;
pub const SCALE_MAX: f64 = 1.60;

/// 曲线宽度可调范围（像素）。
pub const CURVE_WIDTH_MIN: f64 = 0.5;
pub const CURVE_WIDTH_MAX: f64 = 4.0;

/// 字号可调范围（像素，缩放前的基准值）。
pub const FONT_SIZE_MIN: f64 = 8.0;
pub const FONT_SIZE_MAX: f64 = 24.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppConfig {
    /// 配置结构版本。**camelCase**（标准 20 的自测要手改这个字段名）
    #[serde(rename = "schemaVersion", default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default = "default_watchlist")]
    pub watchlist: Vec<WatchItem>,
    #[serde(default)]
    pub appearance: AppearanceConfig,
    #[serde(default)]
    pub window: WindowConfig,
    /// 牛熊提醒（v3 新增）。旧配置缺此字段时由 serde default 补出「全关」，不改变既有行为
    #[serde(default)]
    pub alerts: AlertConfig,
    /// 刷新间隔（2026-08-31 新增）。缺字段时补默认，行为与硬编码时期一致
    #[serde(default)]
    pub refresh: RefreshConfig,
}

/// 数据刷新间隔配置。
///
/// # 为什么单独一段而不是塞进 appearance
///
/// 它既不是外观也不是提醒，塞进现有段落会让语义混乱；
/// 且将来若加"仅交易时段刷新""夜间降频"这类开关，都属于这一段。
///
/// # 为什么有下限
///
/// 报价走腾讯批量接口（一次拿全部自选股），3s 是实测稳定的下限；
/// 分时是**每个标的一个请求**，间隔直接乘以标的数量决定请求速率。
/// 允许用户设到 1s 的话，20 只自选股就是 20 请求/秒，会被上游限流拉黑。
/// 所以 `sanitize()` 会把值钳到安全区间，UI 也只提供区间内的选项。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RefreshConfig {
    /// 报价（价格/涨跌幅）刷新间隔，秒。
    pub quote_secs: u64,
    /// 分时曲线刷新间隔，秒。
    pub minute_secs: u64,
}

impl Default for RefreshConfig {
    fn default() -> Self {
        RefreshConfig {
            // 与调整前的硬编码值一致，保证旧配置升级后行为不变
            quote_secs: 3,
            minute_secs: 5,
        }
    }
}

/// 报价间隔允许区间（秒）。下限 2 是腾讯批量接口的实测安全线；
/// 上限 60 —— 再长就失去"实时看盘"的意义了。
pub const QUOTE_SECS_MIN: u64 = 2;
pub const QUOTE_SECS_MAX: u64 = 60;
/// 分时间隔允许区间（秒）。下限 3 比报价高，因为它是逐标的请求、
/// 请求数会乘以自选股数量。上限 300（5 分钟）——分时本身是分钟级数据。
pub const MINUTE_SECS_MIN: u64 = 3;
pub const MINUTE_SECS_MAX: u64 = 300;

impl RefreshConfig {
    /// 钳到安全区间。配置文件可被手改，进 state 前必须合法。
    pub fn sanitize(&mut self) {
        self.quote_secs = self.quote_secs.clamp(QUOTE_SECS_MIN, QUOTE_SECS_MAX);
        self.minute_secs = self.minute_secs.clamp(MINUTE_SECS_MIN, MINUTE_SECS_MAX);
    }
}

fn default_schema_version() -> u32 {
    SCHEMA_VERSION
}

impl Default for AppConfig {
    fn default() -> Self {
        AppConfig {
            schema_version: SCHEMA_VERSION,
            watchlist: default_watchlist(),
            appearance: AppearanceConfig::default(),
            window: WindowConfig::default(),
            alerts: AlertConfig::default(),
            refresh: RefreshConfig::default(),
        }
    }
}

/// 到价提醒配置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AlertConfig {
    /// 总开关。关掉后不做任何判定、不弹卡片、不出声
    pub enabled: bool,
    /// 上涨提示音总开关。关掉后涨到阈值只弹卡片、不出声
    pub bull_sound: bool,
    /// 下跌提示音总开关
    pub bear_sound: bool,
    /// 上涨音色。内置音色 id（`chime`/`marimba`/`bell`/`drop`/`wood`/`tick`/`none`）
    /// 或 `custom:文件名` 表示用户自选音频（文件在 `config_dir()/sounds/` 下）。
    ///
    /// **不做成枚举**：`custom:` 后面跟任意文件名，枚举表达不了；
    /// 且将来加内置音色时枚举要改两侧（Rust + TS），字符串只需改前端清单。
    /// 合法性由前端校验 + 播放时兜底（认不出的 id 就不出声，不崩）。
    pub bull_tone: String,
    /// 下跌音色，取值同 `bull_tone`
    pub bear_tone: String,
    /// 提示音音量 0.0~1.0
    pub sound_volume: f64,
    /// 提醒卡片不透明度（与悬浮窗三组不透明度相互独立，参考项目也是单列的）
    pub card_opacity: f64,
    /// 同一规则的冷却期（秒）。默认 300s = 5 分钟，防止在阈值边缘反复响
    pub cooldown_secs: u64,
    /// 全局默认涨幅阈值（百分比）。逐股规则未设时用它；`None` = 不启用
    pub default_up_percent: Option<f64>,
    /// 全局默认跌幅阈值（**正数**表示跌幅）。`None` = 不启用
    pub default_down_percent: Option<f64>,
    /// 逐股规则，键是 `tencent_code`（唯一身份键，不是东财裸码）
    pub rules: std::collections::BTreeMap<String, crate::domain::alert::AlertRule>,
}

impl Default for AlertConfig {
    fn default() -> Self {
        AlertConfig {
            // 默认关：不该在用户没设置任何阈值时就开始弹东西
            enabled: false,
            bull_sound: true,
            bear_sound: true,
            // 默认「清铃」：中性、通用、不刺耳。
            // **不用拟声音效**（牛叫/熊叫）—— 用户 2026-08-27 明确不要那种。
            // 涨跌方向靠音高上行/下行区分，不靠动物叫声。
            bull_tone: "chime".to_string(),
            bear_tone: "chime".to_string(),
            sound_volume: 0.7,
            card_opacity: 0.95,
            cooldown_secs: 300,
            default_up_percent: Some(3.0),
            default_down_percent: Some(3.0),
            rules: Default::default(),
        }
    }
}

pub const CARD_OPACITY_MIN: f64 = 0.2;
pub const CARD_OPACITY_MAX: f64 = 1.0;
/// 阈值上限：涨跌幅填到 100% 以上没有实际意义，且会让滞回布防线算出荒谬值
pub const PERCENT_THRESHOLD_MAX: f64 = 100.0;

impl AlertConfig {
    /// 逐字段钳制。配置文件用户可手改，填 `cooldown_secs: 0` 会让阈值边缘每 3s 响一次。
    pub fn sanitize(&mut self) {
        let d = AlertConfig::default();
        self.card_opacity = clamp(
            self.card_opacity,
            CARD_OPACITY_MIN,
            CARD_OPACITY_MAX,
            d.card_opacity,
        );
        // 冷却期至少 5s：再短就失去"防反复"的意义
        self.cooldown_secs = self.cooldown_secs.max(5);

        // 音量钳到 0~1。配置文件用户可手改，填 5.0 会让 WebAudio 输出削波爆音
        self.sound_volume = clamp(self.sound_volume, 0.0, 1.0, d.sound_volume);

        // 音色 id 只做**格式**校验，不校验"是否是已知内置音色" ——
        // 内置清单在前端（`src/lib/sounds.ts`），后端复制一份必然分叉。
        // 前端遇到认不出的 id 会静音而不是崩，所以这里放宽是安全的。
        //
        // 但空串必须修掉：空串会让前端的 `startsWith(CUSTOM_PREFIX)` 与
        // 音色查找双双落空，表现为"选了音色却没声音"且无从排查。
        let fix_tone = |t: &str, fallback: &str| -> String {
            let t = t.trim();
            if t.is_empty() {
                return fallback.to_string();
            }
            // 自定义音频的文件名不允许含路径分隔符 —— 防止配置文件里写
            // `custom:../../../etc/passwd` 之类的路径穿越
            if let Some(name) = t.strip_prefix("custom:") {
                let bad = name.is_empty()
                    || name.contains('/')
                    || name.contains('\\')
                    || name.contains("..");
                if bad {
                    return fallback.to_string();
                }
            }
            t.to_string()
        };
        self.bull_tone = fix_tone(&self.bull_tone, &d.bull_tone);
        self.bear_tone = fix_tone(&self.bear_tone, &d.bear_tone);

        let fix_pct = |v: Option<f64>| -> Option<f64> {
            v.filter(|x| x.is_finite() && *x > 0.0 && *x <= PERCENT_THRESHOLD_MAX)
        };
        self.default_up_percent = fix_pct(self.default_up_percent);
        self.default_down_percent = fix_pct(self.default_down_percent);

        for rule in self.rules.values_mut() {
            rule.up_percent = fix_pct(rule.up_percent);
            rule.down_percent = fix_pct(rule.down_percent);
            // 目标价必须为正且有限；0 或负数视为未设置
            rule.bull_price = rule.bull_price.filter(|x| x.is_finite() && *x > 0.0);
            rule.bear_price = rule.bear_price.filter(|x| x.is_finite() && *x > 0.0);
            // 基准价同规则（它是价格，不是百分比）
            rule.base_price = rule.base_price.filter(|x| x.is_finite() && *x > 0.0);
        }
        // 清掉四项全空的规则，避免配置文件里堆无意义条目
        self.rules.retain(|_, r| !r.is_empty());
    }

    /// 取某只标的的**有效**规则：逐股设置优先，未设的百分比项回落到全局默认。
    pub fn effective_rule(&self, code: &str) -> crate::domain::alert::AlertRule {
        let mut r = self.rules.get(code).cloned().unwrap_or_default();
        if r.up_percent.is_none() {
            r.up_percent = self.default_up_percent;
        }
        if r.down_percent.is_none() {
            r.down_percent = self.default_down_percent;
        }
        r
    }
}

/// 外观配置。三组不透明度**互不嵌套**（D2），故是三个独立字段。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppearanceConfig {
    /// 整体缩放，1.0 = 100%
    pub scale: f64,
    /// 分时曲线线宽（像素）
    pub curve_width: f64,
    /// 基准字号（像素，缩放前）
    pub font_size: f64,
    /// 是否显示证券代码
    pub show_code: bool,
    /// 是否显示市场标识
    pub show_market: bool,
    /// 背景板不透明度。0 = 完全透明（此时文字与曲线仍须清晰可见 —— 标准 11）
    pub opacity_background: f64,
    /// 文字不透明度
    pub opacity_text: f64,
    /// 曲线不透明度
    pub opacity_curve: f64,
    /// 界面主题。`None` = 深色。
    ///
    /// **不需要 schema 迁移**：`Option<Theme>` + 结构体上的 `#[serde(default)]`，
    /// 旧配置缺这个字段时补 `None`。这条有测试证明（`config_migrate.rs` 的
    /// `v5_config_without_theme_loads_as_none`），不靠推理 ——
    /// serde 对 Option 的行为容易记错。
    ///
    /// 只作用于三个普通窗口与中心弹窗；悬浮窗是透明窗口不做主题。
    pub theme: Option<Theme>,
}

/// 界面主题。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Dark,
    Light,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        AppearanceConfig {
            scale: 1.0,
            curve_width: 1.0,
            font_size: 12.0,
            show_code: false,
            show_market: false,
            opacity_background: 0.85,
            opacity_text: 1.0,
            opacity_curve: 1.0,
            // None 而非 Some(Dark)：让"没设置过"和"显式选了深色"在配置文件里可区分，
            // 将来若要加"跟随系统"就不必再改语义
            theme: None,
        }
    }
}

/// 窗口配置。位置是 `Option`：`None` = 用 tauri.conf.json 的初始位置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowConfig {
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub always_on_top: bool,
    /// 锁定 = 不可拖动 + 点击穿透。**解锁的唯一通道是托盘菜单**（标准 16）
    pub locked: bool,
    /// 悬浮窗形态：`true` = 收起为球（单只轮播），`false` = 展开列表。
    ///
    /// **不需要 schema 迁移**：`Option<bool>` + 结构体上的 `#[serde(default)]`，
    /// 旧配置缺这个字段时补 `None`。取值一律走 [`WindowConfig::is_collapsed`]，
    /// 不要直接读这个字段 —— `None` 的语义是"球态"而不是"展开"，
    /// 直接 `unwrap_or_default()` 会得到 `false`（展开），正好反了。
    ///
    /// 为什么 `None` 映射成球态：球是 2026-09-01 起的主形态，
    /// 老用户升级后也该看到球（用户要的是"默认球、点击才展开"）。
    pub collapsed: Option<bool>,
}

impl WindowConfig {
    /// 悬浮窗是否处于球态。`None`（旧配置/未设置）视为 `true`。
    ///
    /// 存在的理由：`collapsed` 是 `Option<bool>`，而 `None` 的业务语义是"球态"，
    /// 与 `bool::default()`（false）相反。所有读取点都必须经过这里，
    /// 否则会出现"旧配置升级后直接展开成列表"的回归。
    pub fn is_collapsed(&self) -> bool {
        self.collapsed.unwrap_or(true)
    }
}

impl Default for WindowConfig {
    fn default() -> Self {
        WindowConfig {
            x: None,
            y: None,
            always_on_top: true,
            locked: false,
            // None 而不是 Some(true)：让"未写过这个字段"和"显式设为球态"在
            // 配置文件里可区分。读取一律走 is_collapsed()，两者行为相同。
            collapsed: None,
        }
    }
}

/// 把数值钳制到合法区间。配置文件是用户可手改的文本，
/// 手填 `scale: 99` 不该让窗口变成 99 倍大（也不该崩）。
fn clamp(v: f64, lo: f64, hi: f64, fallback: f64) -> f64 {
    if !v.is_finite() {
        // NaN / Infinity（JSON 里可由 1e999 得到）→ 回落默认值
        return fallback;
    }
    v.clamp(lo, hi)
}

impl AppearanceConfig {
    /// 逐字段钳制到合法区间。在加载与每次更新时都调用，
    /// 保证进入 state 的配置**永远合法**（前端滑块也就不必重复校验）。
    pub fn sanitize(&mut self) {
        let d = AppearanceConfig::default();
        self.scale = clamp(self.scale, SCALE_MIN, SCALE_MAX, d.scale);
        self.curve_width = clamp(
            self.curve_width,
            CURVE_WIDTH_MIN,
            CURVE_WIDTH_MAX,
            d.curve_width,
        );
        self.font_size = clamp(self.font_size, FONT_SIZE_MIN, FONT_SIZE_MAX, d.font_size);
        self.opacity_background = clamp(self.opacity_background, 0.0, 1.0, d.opacity_background);
        self.opacity_text = clamp(self.opacity_text, 0.0, 1.0, d.opacity_text);
        self.opacity_curve = clamp(self.opacity_curve, 0.0, 1.0, d.opacity_curve);
    }
}

impl AppConfig {
    /// 加载后的统一净化：外观钳制 + 自选股去重 + 超上限截断。
    ///
    /// 自选股去重按 `tencent_code`（唯一键）；**不是**按东财裸码，
    /// 否则 `sz000001` 与 `sh000001` 会被误当成同一条而丢掉一只。
    pub fn sanitize(&mut self) {
        self.appearance.sanitize();
        self.alerts.sanitize();
        // 刷新间隔必须钳到安全区间：配置文件可被手改，
        // 手写 minute_secs=1 会让 20 只自选股变成 20 请求/秒，被上游限流拉黑
        self.refresh.sanitize();

        let mut seen: Vec<String> = Vec::new();
        let mut kept: Vec<WatchItem> = Vec::new();
        for it in self.watchlist.drain(..) {
            let k = it.tencent_code.to_ascii_lowercase();
            if k.is_empty() || seen.contains(&k) {
                continue;
            }
            if kept.len() >= crate::watchlist::MAX_WATCH_ITEMS {
                break;
            }
            seen.push(k);
            kept.push(it);
        }
        self.watchlist = kept;
    }
}
