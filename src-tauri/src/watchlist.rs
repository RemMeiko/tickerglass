use crate::domain::market::Market;
use crate::model::WatchItem;

/// 自选股上限（spec 标准 7）。
pub const MAX_WATCH_ITEMS: usize = 50;

/// 腾讯报价一批最多 40 个代码（已验证的接口事实）。
pub const QUOTE_BATCH_SIZE: usize = 40;

/// 首次启动的默认自选股，覆盖指数 / A股 / 港股 / 美股四类市场。
///
/// Sprint 2A 起它从"运行时数据源"降级为"首次启动的默认配置"（D1）：
/// `scheduler` 不再直接读它，改读 `AppState` 里的配置快照。
///
/// 由 `const` 改为 `fn` 的原因：字段已是 `String`，**`String` 无法 const 构造**。
pub fn default_watchlist() -> Vec<WatchItem> {
    vec![
        // 默认四只全部可见 —— 首次启动就有东西可看
        WatchItem {
            tencent_code: "sh000001".to_string(),
            em_secid: Some("1.000001".to_string()),
            market: Market::IndexCn,
            is_index: true,
            hidden: false,
        },
        WatchItem {
            tencent_code: "sh600519".to_string(),
            em_secid: Some("1.600519".to_string()),
            market: Market::Ashare,
            is_index: false,
            hidden: false,
        },
        WatchItem {
            tencent_code: "hk00700".to_string(),
            em_secid: Some("116.00700".to_string()),
            market: Market::Hk,
            is_index: false,
            hidden: false,
        },
        WatchItem {
            tencent_code: "usAAPL".to_string(),
            em_secid: Some("105.AAPL".to_string()),
            market: Market::Us,
            is_index: false,
            hidden: false,
        },
    ]
}

/// 添加自选股失败的原因。做成枚举而不是布尔，让前端能给出不同提示文案。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddError {
    /// 已达 50 只上限
    LimitReached,
    /// 该标的已在自选股中（按 `tencent_code` 判定）
    Duplicate,
}

impl std::fmt::Display for AddError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AddError::LimitReached => {
                write!(f, "已达 {MAX_WATCH_ITEMS} 只上限")
            }
            AddError::Duplicate => write!(f, "已在自选股中"),
        }
    }
}

/// `tencent_code` 的规范化比较键。
///
/// 腾讯代码的市场前缀大小写不敏感（`usAAPL` / `USAAPL` 同一标的），
/// 但美股票代码本身习惯大写。统一用小写做键，避免同一标的被当成两条。
fn key(tencent_code: &str) -> String {
    tencent_code.to_ascii_lowercase()
}

/// 是否已存在该标的。**按 `tencent_code` 判定，不是按东财裸码。**
pub fn contains(list: &[WatchItem], tencent_code: &str) -> bool {
    let k = key(tencent_code);
    list.iter().any(|it| key(&it.tencent_code) == k)
}

/// 追加一个标的到**末尾**（spec 标准 2 要求新标的出现在悬浮窗底部）。
///
/// 纯函数：返回新列表，不改入参。上限与去重都在这里判定。
pub fn add(list: &[WatchItem], item: WatchItem) -> Result<Vec<WatchItem>, AddError> {
    if contains(list, &item.tencent_code) {
        // 去重先于上限判定：50 只满了再点已有标的，应提示"已在自选股中"而不是"已达上限"
        return Err(AddError::Duplicate);
    }
    if list.len() >= MAX_WATCH_ITEMS {
        return Err(AddError::LimitReached);
    }
    let mut next = list.to_vec();
    next.push(item);
    Ok(next)
}

/// 移除一个标的。**按 `tencent_code` 移除。**
///
/// ⚠️ 这是整个模块最容易写错的地方：若按东财裸码 `000001` 移除，
/// 会同时删掉平安银行(`sz000001`)与上证指数(`sh000001`)。
///
/// 返回 `(新列表, 是否真的删掉了一条)`。
pub fn remove(list: &[WatchItem], tencent_code: &str) -> (Vec<WatchItem>, bool) {
    let k = key(tencent_code);
    let next: Vec<WatchItem> = list
        .iter()
        .filter(|it| key(&it.tencent_code) != k)
        .cloned()
        .collect();
    let removed = next.len() != list.len();
    (next, removed)
}

/// 按给定的 `tencent_code` 顺序重排列表。
///
/// 容错设计（拖拽排序的前端可能传来不完整的序列）：
/// - `order` 中不存在于 `list` 的代码被忽略
/// - `list` 中未出现在 `order` 里的条目**按原相对顺序追加到末尾**（不丢项）
pub fn reorder(list: &[WatchItem], order: &[String]) -> Vec<WatchItem> {
    let mut next: Vec<WatchItem> = Vec::with_capacity(list.len());
    let mut taken = vec![false; list.len()];

    for code in order {
        let k = key(code);
        if let Some(idx) = list
            .iter()
            .enumerate()
            .position(|(i, it)| !taken[i] && key(&it.tencent_code) == k)
        {
            taken[idx] = true;
            next.push(list[idx].clone());
        }
    }
    // 未被 order 覆盖的条目不丢，按原顺序补到末尾
    for (i, it) in list.iter().enumerate() {
        if !taken[i] {
            next.push(it.clone());
        }
    }
    next
}

/// 把代码集切成腾讯报价可接受的批次（一批最多 40 个）。
///
/// 独立成纯函数是为了让「39/40/41/50 只各分几批」可被单测锁住 ——
/// 50 只上限下不分批会直接超腾讯上限，而这个 bug 只在用户加满时才暴露。
///
/// 0 个代码返回空 `Vec`，调用方据此**不发请求**。
pub fn batch_codes(codes: &[String]) -> Vec<Vec<String>> {
    if codes.is_empty() {
        return Vec::new();
    }
    codes
        .chunks(QUOTE_BATCH_SIZE)
        .map(|c| c.to_vec())
        .collect()
}
