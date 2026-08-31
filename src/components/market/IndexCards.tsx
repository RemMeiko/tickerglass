import type { IndexQuote } from "../../types";
import { formatMoney, formatPercent, trendClass } from "../../lib/formatMoney";

interface Props {
  indices: IndexQuote[];
  /** 非 null 时指数整体没拿到 —— 与「三条都平盘」是两回事，必须分开显示 */
  error: string | null;
  /** 取得时刻。轮转取数下指数与各市场的新鲜度不同，必须分别标注 */
  fetchedAt?: string | null;
}

export default function IndexCards({ indices, error, fetchedAt }: Props) {
  if (error) {
    return (
      <section className="mk-section">
        <h2 className="mk-section-title">指数</h2>
        <div className="mk-hint mk-warn" role="status">
          {error}
        </div>
      </section>
    );
  }

  return (
    <section className="mk-section">
      <h2 className="mk-section-title">
        指数
        {fetchedAt && <span className="mk-section-sub">{fetchedAt}</span>}
      </h2>
      <div className="idx-cards">
        {indices.map((q) => {
          const t = trendClass(q.change_percent);
          return (
            <div key={q.code} className={`idx-card idx-${t}`}>
              <div className="idx-name">{q.name}</div>
              {/* 点位小数位固定 2 位：指数报文本身就是 2 位 */}
              <div className="idx-price">{q.price.toFixed(2)}</div>
              <div className="idx-change">
                <span className="idx-pct">{formatPercent(q.change_percent)}</span>
                <span className="idx-amt">
                  {q.change_amount > 0 ? "+" : ""}
                  {q.change_amount.toFixed(2)}
                </span>
              </div>
              {/* 成交额可能缺失（f6 不总是返回），缺了就不显示这一行而不是显示 0 */}
              {q.amount !== null && (
                <div className="idx-turnover">成交 {formatMoney(q.amount, 0)}</div>
              )}
            </div>
          );
        })}
      </div>
    </section>
  );
}
