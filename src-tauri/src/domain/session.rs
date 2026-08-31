use super::market::Market;
use crate::model::{MinutePoint, RawPoint};
use std::sync::OnceLock;

/// 生成 `[from, to]` 闭区间的分钟刻度（`hhmm` 形式，如 930 / 1130）。
fn ticks(from: u16, to: u16) -> Vec<u16> {
    let (mut h, mut m) = (from / 100, from % 100);
    let (eh, em) = (to / 100, to % 100);
    let mut out = Vec::new();
    loop {
        out.push(h * 100 + m);
        if h == eh && m == em {
            break;
        }
        m += 1;
        if m == 60 {
            m = 0;
            h += 1;
        }
        if h > 23 {
            break;
        }
    }
    out
}

/// 栅格 = 刻度序列 + 每个刻度所属的交易段序号。
///
/// 段序号是午休断线的唯一依据：栅格**故意不给午休留格**（否则 X 轴中间会空出
/// 90 分钟的死区），所以 A 股 `1130`(idx 120) 与 `1300`(idx 121) 在栅格上是
/// **相邻**的。仅靠 `grid_index` 连续性判断永远检测不到午休，必须靠段序号。
struct Grid {
    ticks: Vec<u16>,
    segment_of: Vec<u8>,
}

fn build(segments: &[(u16, u16)]) -> Grid {
    let mut all_ticks = Vec::new();
    let mut segment_of = Vec::new();
    for (seg, &(a, b)) in segments.iter().enumerate() {
        let t = ticks(a, b);
        segment_of.extend(std::iter::repeat(seg as u8).take(t.len()));
        all_ticks.extend(t);
    }
    Grid {
        ticks: all_ticks,
        segment_of,
    }
}

static GRID_CN: OnceLock<Grid> = OnceLock::new();
static GRID_HK: OnceLock<Grid> = OnceLock::new();
static GRID_US: OnceLock<Grid> = OnceLock::new();

fn grid_of(market: Market) -> &'static Grid {
    match market {
        Market::Ashare | Market::IndexCn => {
            GRID_CN.get_or_init(|| build(&[(930, 1130), (1300, 1500)]))
        }
        Market::Hk => GRID_HK.get_or_init(|| build(&[(930, 1200), (1300, 1600)])),
        Market::Us => GRID_US.get_or_init(|| build(&[(930, 1600)])),
    }
}

/// 该市场的交易分钟栅格（静态时间轴定义，与当日是否真有成交无关）。
///
/// 长度是 X 轴映射的分母（`X = grid_index / (len - 1)`），差 1 会让整条曲线
/// 横向系统性偏移，故三个长度由单测断言钉死，不靠人算。
///
/// - A 股/指数：09:30–11:30 + 13:00–15:00 = 121 + 121 = **242**
/// - 港股：09:30–12:00 + 13:00–16:00 = 151 + 181 = **332**（午休在 12:00–13:00）
/// - 美股：09:30–16:00 美东连续 = **391**（东财返回的北京时间已由 `parse::em_trends` 换算）
pub fn grid(market: Market) -> &'static [u16] {
    &grid_of(market).ticks
}

pub fn grid_len(market: Market) -> usize {
    grid(market).len()
}

/// `hhmm` 在该市场栅格中的位置。栅格外（午休中、盘前、盘后）返回 `None`。
pub fn index_of(market: Market, hhmm: u16) -> Option<usize> {
    grid(market).iter().position(|&t| t == hhmm)
}

/// 该刻度所属的交易段序号（上午 0 / 下午 1）。栅格外返回 `None`。
fn segment_at(market: Market, grid_index: usize) -> Option<u8> {
    grid_of(market).segment_of.get(grid_index).copied()
}

/// 把原始点序列落到栅格上：
/// 1. 栅格外的点（A 股 15:06–15:30、港股 16:01–16:08 等盘后重复点）**直接丢弃** ——
///    它们的价格全等于收盘价，留下来会污染 Y 轴极值并在曲线尾部拖出平直尾巴
/// 2. 计算 `grid_index`（前端 X 轴映射的唯一依据）
/// 3. 计算 `break_before`：跨交易段（午休）或缺点造成 `grid_index` 跳跃时为 `true`，
///    前端只按这个标志 `moveTo`，因此前端不需要知道任何市场的午休时间
///
/// 不做任何补点/插值 —— 点不够就是不够。
pub fn attach_grid(market: Market, raw: &[RawPoint]) -> Vec<MinutePoint> {
    let mut out: Vec<MinutePoint> = Vec::with_capacity(raw.len());
    let mut prev: Option<(usize, Option<u8>)> = None;

    for p in raw {
        let Some(idx) = index_of(market, p.hhmm) else {
            continue; // 栅格外，丢弃
        };
        // 价格无效（停牌 0 价 / 解析不出）不进曲线，也不参与极值
        if !(p.price > 0.0) || !p.price.is_finite() {
            continue;
        }
        let seg = segment_at(market, idx);
        let break_before = match prev {
            None => false, // 序列首点恒为 false
            Some((prev_idx, prev_seg)) => {
                // 跨交易段（午休两端在栅格上相邻，只能靠段号识别），
                // 或本段内 grid_index 跳跃（中间有缺点）
                prev_seg != seg || idx != prev_idx + 1
            }
        };
        out.push(MinutePoint {
            hhmm: p.hhmm,
            grid_index: idx,
            price: p.price,
            avg: p.avg.filter(|v| v.is_finite() && *v > 0.0),
            break_before,
        });
        prev = Some((idx, seg));
    }
    out
}
