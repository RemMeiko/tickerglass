import type { BreadthTotal, MarketRow } from "../../types";

interface Props {
  total: BreadthTotal;
  /** 逐市场明细，用来在某个市场失败时说清楚缺的是哪个 */
  markets: MarketRow[];
}

/** A 股共三个交易所（沪/深/北），少于这个数就是数据不全 */
const EXPECTED_MARKETS = 3;

export default function BreadthBar({ total, markets }: Props) {
  const sum = total.up + total.down + total.flat;
  const failed = markets.filter((m) => m.stats === null);

  // markets_counted 为 0 = 三个市场全挂。此时家数都是 0，
  // 但绝不能显示「涨 0 跌 0」—— 那是假数据，必须显示成拿不到
  if (total.markets_counted === 0) {
    return (
      <section className="mk-section">
        <h2 className="mk-section-title">涨跌家数</h2>
        <div className="mk-hint mk-warn" role="status">
          三个市场的家数都没拿到
          {failed[0]?.error ? `（${failed[0].error}）` : ""}
        </div>
      </section>
    );
  }

  const pct = (n: number) => (sum > 0 ? (n / sum) * 100 : 0);

  return (
    <section className="mk-section">
      <h2 className="mk-section-title">
        涨跌家数
        <span className="mk-section-sub">共 {sum} 只</span>
      </h2>

      {/* 数据不全时必须显式标注：否则用户会把「沪+深」当成全 A 股，
          少算北交所约 338 只，而界面上看不出任何异常 */}
      {total.markets_counted < EXPECTED_MARKETS && (
        <div className="mk-hint mk-warn" role="status">
          数据不全：只统计了 {total.markets_counted} / {EXPECTED_MARKETS} 个市场
          {failed.length > 0 && `（缺 ${failed.map((m) => m.label).join("、")}）`}
        </div>
      )}

      <div
        className="bd-bar"
        role="img"
        aria-label={`上涨 ${total.up} 家，下跌 ${total.down} 家，平盘 ${total.flat} 家`}
      >
        {/* 三段宽度按占比；0 家时该段不渲染，避免出现 0 宽度的边框缝 */}
        {total.up > 0 && (
          <div className="bd-seg bd-up" style={{ width: `${pct(total.up)}%` }} />
        )}
        {total.flat > 0 && (
          <div className="bd-seg bd-flat" style={{ width: `${pct(total.flat)}%` }} />
        )}
        {total.down > 0 && (
          <div className="bd-seg bd-down" style={{ width: `${pct(total.down)}%` }} />
        )}
      </div>

      <div className="bd-legend">
        <span className="bd-item bd-t-up">
          <i className="bd-dot" />涨 {total.up}
        </span>
        <span className="bd-item bd-t-flat">
          <i className="bd-dot" />平 {total.flat}
        </span>
        <span className="bd-item bd-t-down">
          <i className="bd-dot" />跌 {total.down}
        </span>
      </div>

      {/* 逐市场明细：合计之外还给出分解，便于用户判断是哪个市场在拖 */}
      <ul className="bd-markets">
        {markets.map((m) => (
          <li key={m.secid} className="bd-mkt-row">
            <span className="bd-mkt-label">{m.label}</span>
            {m.stats ? (
              <span className="bd-mkt-nums">
                <span className="bd-t-up">{m.stats.breadth.up}</span>
                <span className="bd-sep">/</span>
                <span className="bd-t-flat">{m.stats.breadth.flat}</span>
                <span className="bd-sep">/</span>
                <span className="bd-t-down">{m.stats.breadth.down}</span>
                {/* 逐行标时刻：轮转取数下三个市场的新鲜度不同，
                    不标会让用户以为这三组数字是同一时刻的 */}
                {m.fetched_at && (
                  <span className="bd-mkt-time">{m.fetched_at}</span>
                )}
              </span>
            ) : (
              <span className="bd-mkt-err">{m.error ?? "无数据"}</span>
            )}
          </li>
        ))}
      </ul>
    </section>
  );
}
