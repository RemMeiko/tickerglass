//! 自选股增删改序（纯函数）+ 报价分批切分边界。
//!
//! 核心防线：**身份键是 `tencent_code`，不是东财裸码**。
//! 误按裸码删会让 `remove("000001")` 同时删掉平安银行与上证指数。

use tickerglass_lib::domain::market::Market;
use tickerglass_lib::model::WatchItem;
use tickerglass_lib::watchlist::{
    add, batch_codes, contains, default_watchlist, remove, reorder, AddError, MAX_WATCH_ITEMS,
    QUOTE_BATCH_SIZE,
};

fn item(code: &str, market: Market, is_index: bool) -> WatchItem {
    WatchItem {
        tencent_code: code.to_string(),
        em_secid: Some(format!("1.{code}")),
        market,
        is_index,
        // 默认可见。隐藏是独立维度，与增删改序互不影响 ——
        // `add`/`remove`/`reorder` 都不该读写它（下面有专门的测试锁这一点）
        hidden: false,
    }
}

fn pingan() -> WatchItem {
    item("sz000001", Market::Ashare, false)
}

fn szzs() -> WatchItem {
    item("sh000001", Market::IndexCn, true)
}

fn codes_of(list: &[WatchItem]) -> Vec<String> {
    list.iter().map(|i| i.tencent_code.clone()).collect()
}

// ---------- 1. 默认列表 ----------

#[test]
fn default_watchlist_has_four_items() {
    assert_eq!(default_watchlist().len(), 4);
}

#[test]
fn default_watchlist_covers_four_markets() {
    let l = default_watchlist();
    let markets: Vec<Market> = l.iter().map(|i| i.market).collect();
    assert!(markets.contains(&Market::IndexCn));
    assert!(markets.contains(&Market::Ashare));
    assert!(markets.contains(&Market::Hk));
    assert!(markets.contains(&Market::Us));
}

#[test]
fn default_watchlist_all_have_known_secid() {
    // 默认四只的 secid 都是已验证过的真值，不该有 None
    assert!(default_watchlist().iter().all(|i| i.em_secid.is_some()));
}

// ---------- 2. add：追加到末尾 + 上限 + 去重 ----------

#[test]
fn add_appends_to_the_end() {
    // 标准 2 要求新标的出现在悬浮窗**底部**
    let list = default_watchlist();
    let next = add(&list, pingan()).expect("应添加成功");
    assert_eq!(next.len(), 5);
    assert_eq!(next.last().unwrap().tencent_code, "sz000001");
}

#[test]
fn add_does_not_mutate_the_input_list() {
    let list = default_watchlist();
    let _ = add(&list, pingan()).unwrap();
    assert_eq!(list.len(), 4, "纯函数不得改入参");
}

#[test]
fn add_rejects_duplicate_identity() {
    let list = default_watchlist();
    let dup = item("sh600519", Market::Ashare, false);
    assert_eq!(add(&list, dup), Err(AddError::Duplicate));
}

#[test]
fn add_duplicate_check_is_case_insensitive_on_prefix() {
    // usAAPL 与 USAAPL 是同一标的，不该被当成两条
    let list = default_watchlist();
    let dup = item("USAAPL", Market::Us, false);
    assert_eq!(add(&list, dup), Err(AddError::Duplicate));
}

#[test]
fn add_allows_same_bare_code_in_different_markets() {
    // sz000001(平安银行) 与 sh000001(上证指数) 是两个标的，都要能加
    let list = vec![pingan()];
    let next = add(&list, szzs()).expect("不同身份键不是重复项");
    assert_eq!(next.len(), 2);
}

#[test]
fn add_rejects_when_limit_reached() {
    let list: Vec<WatchItem> = (0..MAX_WATCH_ITEMS)
        .map(|i| item(&format!("sh6{:05}", i), Market::Ashare, false))
        .collect();
    let r = add(&list, pingan());
    assert_eq!(r, Err(AddError::LimitReached));
}

#[test]
fn add_succeeds_at_exactly_one_below_limit() {
    let list: Vec<WatchItem> = (0..MAX_WATCH_ITEMS - 1)
        .map(|i| item(&format!("sh6{:05}", i), Market::Ashare, false))
        .collect();
    let next = add(&list, pingan()).expect("第 50 只应能加入");
    assert_eq!(next.len(), MAX_WATCH_ITEMS);
}

#[test]
fn duplicate_check_precedes_limit_check() {
    // 满 50 只时再点已有标的，应提示"已在自选股中"而不是"已达上限"
    let mut list: Vec<WatchItem> = (0..MAX_WATCH_ITEMS - 1)
        .map(|i| item(&format!("sh6{:05}", i), Market::Ashare, false))
        .collect();
    list.push(pingan());
    assert_eq!(list.len(), MAX_WATCH_ITEMS);
    assert_eq!(add(&list, pingan()), Err(AddError::Duplicate));
}

#[test]
fn limit_constant_is_fifty() {
    assert_eq!(MAX_WATCH_ITEMS, 50);
}

#[test]
fn add_error_messages_are_distinguishable() {
    assert!(AddError::LimitReached.to_string().contains("50"));
    assert!(AddError::Duplicate.to_string().contains("已在自选股"));
}

// ---------- 3. remove：按身份键，不按裸码（本模块最重要的一组）----------

#[test]
fn remove_deletes_only_the_matching_identity() {
    let list = vec![pingan(), szzs()];
    let (next, removed) = remove(&list, "sz000001");
    assert!(removed);
    assert_eq!(next.len(), 1);
    assert_eq!(next[0].tencent_code, "sh000001");
}

#[test]
fn removing_pingan_keeps_shanghai_index() {
    // 这是合同点名要防的 bug：误按裸码 000001 删会把两条一起删掉
    let list = vec![pingan(), szzs()];
    let (next, _) = remove(&list, "sz000001");
    assert!(
        next.iter().any(|i| i.tencent_code == "sh000001"),
        "删平安银行不得连带删掉上证指数"
    );
}

#[test]
fn removing_index_keeps_pingan() {
    // 反向再锁一次
    let list = vec![pingan(), szzs()];
    let (next, _) = remove(&list, "sh000001");
    assert_eq!(next.len(), 1);
    assert_eq!(next[0].tencent_code, "sz000001");
}

#[test]
fn remove_by_bare_code_matches_nothing() {
    // 裸码不是身份键，用它删应该删不动任何东西（而不是删掉两条）
    let list = vec![pingan(), szzs()];
    let (next, removed) = remove(&list, "000001");
    assert!(!removed, "裸码不该匹配到任何标的");
    assert_eq!(next.len(), 2, "两条都必须还在");
}

#[test]
fn remove_is_case_insensitive() {
    let list = default_watchlist();
    let (next, removed) = remove(&list, "USAAPL");
    assert!(removed);
    assert_eq!(next.len(), 3);
}

#[test]
fn remove_nonexistent_code_is_a_noop() {
    let list = default_watchlist();
    let (next, removed) = remove(&list, "sh999999");
    assert!(!removed);
    assert_eq!(next.len(), 4);
}

#[test]
fn remove_middle_item_preserves_neighbors_order() {
    // 标准 4：删中间一行不影响上下行
    let list = default_watchlist();
    let (next, _) = remove(&list, "sh600519");
    assert_eq!(codes_of(&next), vec!["sh000001", "hk00700", "usAAPL"]);
}

#[test]
fn remove_all_items_yields_empty_list() {
    let mut list = default_watchlist();
    for code in ["sh000001", "sh600519", "hk00700", "usAAPL"] {
        let (next, removed) = remove(&list, code);
        assert!(removed);
        list = next;
    }
    assert!(list.is_empty());
}

// ---------- 4. contains ----------

#[test]
fn contains_matches_by_identity_not_bare_code() {
    let list = vec![pingan()];
    assert!(contains(&list, "sz000001"));
    assert!(!contains(&list, "000001"), "裸码不是身份键");
    assert!(!contains(&list, "sh000001"), "另一个市场是另一个标的");
}

// ---------- 5. reorder ----------

#[test]
fn reorder_moves_last_item_to_first() {
    // 标准 5 的主场景
    let list = default_watchlist();
    let order: Vec<String> = vec!["usAAPL", "sh000001", "sh600519", "hk00700"]
        .into_iter()
        .map(String::from)
        .collect();
    let next = reorder(&list, &order);
    assert_eq!(codes_of(&next), order);
}

#[test]
fn reorder_with_identical_order_is_a_noop() {
    // 标准 5 边界：原地放下，顺序不变且不报错
    let list = default_watchlist();
    let order = codes_of(&list);
    let next = reorder(&list, &order);
    assert_eq!(codes_of(&next), order);
}

#[test]
fn reorder_with_empty_order_keeps_everything() {
    // 拖到列表外空白区域：不丢项
    let list = default_watchlist();
    let next = reorder(&list, &[]);
    assert_eq!(next.len(), 4, "空顺序不得丢项");
    assert_eq!(codes_of(&next), codes_of(&list), "应保持原顺序");
}

#[test]
fn reorder_ignores_unknown_codes() {
    let list = default_watchlist();
    let order: Vec<String> = vec!["sh999999", "usAAPL"].into_iter().map(String::from).collect();
    let next = reorder(&list, &order);
    assert_eq!(next.len(), 4, "不认识的代码被忽略，但不丢已有项");
    assert_eq!(next[0].tencent_code, "usAAPL");
}

#[test]
fn reorder_appends_items_missing_from_order() {
    // 前端只传了部分代码：其余按原相对顺序补到末尾，一条不丢
    let list = default_watchlist();
    let order: Vec<String> = vec!["hk00700".to_string()];
    let next = reorder(&list, &order);
    assert_eq!(next.len(), 4);
    assert_eq!(next[0].tencent_code, "hk00700");
    assert_eq!(
        codes_of(&next)[1..],
        ["sh000001", "sh600519", "usAAPL"]
    );
}

#[test]
fn reorder_is_case_insensitive() {
    let list = default_watchlist();
    let order: Vec<String> = vec!["USAAPL".to_string()];
    let next = reorder(&list, &order);
    assert_eq!(next[0].tencent_code, "usAAPL");
}

#[test]
fn reorder_does_not_duplicate_on_repeated_code() {
    // order 里同一代码出现两次：不该复制出两条
    let list = default_watchlist();
    let order: Vec<String> = vec!["usAAPL", "usAAPL"].into_iter().map(String::from).collect();
    let next = reorder(&list, &order);
    assert_eq!(next.len(), 4);
    assert_eq!(
        next.iter().filter(|i| i.tencent_code == "usAAPL").count(),
        1
    );
}

// ---------- 6. 分批切分边界（Evaluator N-3）----------

fn n_codes(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("sh6{:05}", i)).collect()
}

#[test]
fn batch_size_constant_is_forty() {
    assert_eq!(QUOTE_BATCH_SIZE, 40);
}

#[test]
fn zero_codes_produce_no_batches_so_no_request_is_sent() {
    // 0 只时**不发请求**（而不是发一个 `q=` 空查询）
    assert!(batch_codes(&[]).is_empty());
}

#[test]
fn thirty_nine_codes_fit_in_one_batch() {
    assert_eq!(batch_codes(&n_codes(39)).len(), 1);
}

#[test]
fn exactly_forty_codes_fit_in_one_batch() {
    // 边界：40 是上限本身，不该切成两批
    assert_eq!(batch_codes(&n_codes(40)).len(), 1);
}

#[test]
fn forty_one_codes_split_into_two_batches() {
    let b = batch_codes(&n_codes(41));
    assert_eq!(b.len(), 2);
    assert_eq!(b[0].len(), 40);
    assert_eq!(b[1].len(), 1);
}

#[test]
fn fifty_codes_split_into_two_batches() {
    // 标准 7 的实际场景：50 只上限
    let b = batch_codes(&n_codes(50));
    assert_eq!(b.len(), 2);
    assert_eq!(b[0].len(), 40);
    assert_eq!(b[1].len(), 10);
}

#[test]
fn batching_preserves_order_and_loses_nothing() {
    let all = n_codes(50);
    let flat: Vec<String> = batch_codes(&all).into_iter().flatten().collect();
    assert_eq!(flat, all, "分批不得丢码或改序");
}

#[test]
fn single_code_produces_one_batch() {
    assert_eq!(batch_codes(&n_codes(1)).len(), 1);
}

// ---------- 隐藏与增删改序的正交性 ----------

/// 造一个隐藏的条目。
fn hidden_item(code: &str) -> WatchItem {
    WatchItem {
        hidden: true,
        ..item(code, Market::Ashare, false)
    }
}

#[test]
fn hidden_item_still_counts_as_contained() {
    // 隐藏 ≠ 不存在。若 contains 漏掉隐藏项，用户会被允许重复添加同一只，
    // 于是列表里出现两条相同代码 —— 而身份键本该唯一。
    let list = vec![hidden_item("sh600519")];
    assert!(contains(&list, "sh600519"), "隐藏的标的仍应算已在自选股中");
}

#[test]
fn adding_duplicate_of_hidden_item_is_rejected() {
    let list = vec![hidden_item("sh600519")];
    let r = add(&list, item("sh600519", Market::Ashare, false));
    assert_eq!(
        r,
        Err(AddError::Duplicate),
        "已隐藏的标的再次添加应报重复，而不是加出第二条"
    );
}

#[test]
fn hidden_item_occupies_a_slot_against_the_limit() {
    // 隐藏不释放名额：它仍在配置里、仍在取数。
    // 若隐藏项不计数，用户能加超过 50 只，超出腾讯批量上限的保护就失效了。
    let mut list: Vec<WatchItem> = (0..MAX_WATCH_ITEMS)
        .map(|i| hidden_item(&format!("sh{:06}", 600000 + i)))
        .collect();
    assert_eq!(list.len(), MAX_WATCH_ITEMS);
    let r = add(&list, item("sh999999", Market::Ashare, false));
    assert_eq!(r, Err(AddError::LimitReached), "隐藏项同样占用名额");
    // 删掉一个隐藏项后又能加了
    let (shorter, removed) = remove(&list, "sh600000");
    assert!(removed);
    list = shorter;
    assert!(add(&list, item("sh999999", Market::Ashare, false)).is_ok());
}

#[test]
fn remove_works_on_hidden_items() {
    // 隐藏的标的必须仍可删除 —— 否则藏起来就再也删不掉了
    let list = vec![hidden_item("sh600519"), item("hk00700", Market::Hk, false)];
    let (next, removed) = remove(&list, "sh600519");
    assert!(removed);
    assert_eq!(next.len(), 1);
    assert_eq!(next[0].tencent_code, "hk00700");
}

#[test]
fn reorder_preserves_hidden_flags() {
    // 重排只动顺序，**绝不能重置隐藏状态** ——
    // 用户拖一下顺序结果藏起来的股票全冒出来，是很突兀的 bug。
    let list = vec![
        item("sh600519", Market::Ashare, false),
        hidden_item("hk00700"),
        item("usAAPL", Market::Us, false),
    ];
    let order = vec![
        "usAAPL".to_string(),
        "hk00700".to_string(),
        "sh600519".to_string(),
    ];
    let next = reorder(&list, &order);
    assert_eq!(next[0].tencent_code, "usAAPL");
    assert_eq!(next[1].tencent_code, "hk00700");
    assert!(next[1].hidden, "重排后 hk00700 仍应是隐藏的");
    assert!(!next[0].hidden);
    assert!(!next[2].hidden);
}

#[test]
fn add_preserves_hidden_flag_of_existing_items() {
    // 添加新标的不该影响已有项的隐藏状态
    let list = vec![hidden_item("sh600519")];
    let next = add(&list, item("hk00700", Market::Hk, false)).expect("应添加成功");
    assert_eq!(next.len(), 2);
    assert!(next[0].hidden, "已有的隐藏项不该被新增操作重置");
    assert!(!next[1].hidden, "新添加的标的默认可见");
}

#[test]
fn batch_codes_includes_hidden_items() {
    // 隐藏语义是"不显示"而非"不取数"（用户 2026-08-28 选定 B 方案）：
    // 后台照常拉行情、照常判定提醒，取消隐藏后才能立刻有数据。
    // 所以切批时必须**包含**隐藏项。
    let codes: Vec<String> = vec!["sh600519".into(), "hk00700".into()];
    let batches = batch_codes(&codes);
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].len(), 2, "隐藏项也要进请求批次");
}
