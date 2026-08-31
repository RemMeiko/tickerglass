import type { MarketRow, MoneyFlow } from "../../types";
import { formatSignedMoney, trendClass } from "../../lib/formatMoney";

interface Props {
  markets: MarketRow[];
  /** 三市场主力净流入合计（元） */
  mainForceNet: number;
}

/** 五档的显示顺序：由大到小，与东财资金流向页一致 */
const TIERS: { key: keyof MoneyFlow; label: string }[] = [
  { key: "extra_large", label: "超大单" },
  { key: "large", label: "大单" },
  { key: "medium", label: "中单" },
  { key: "small", label: "小单" },
];

export default function FlowPanel({ markets, mainForceNet }: Props) {
  const ok = markets.filter((m) => m.stats !== null);

  if (ok.length === 0) {
    return (
      <section className="mk-section">
        <h2 className="mk-section-title">资金流向</h2>
        <div className="mk-hint mk-warn" role="status">
          没拿到任何市场的资金流向
        </div>
      </section>
    );
  }

  // 各档跨市场合计。只加净额 —— 跨市场的"流入总额"没有解释意义
  const tierTotals = TIERS.map((t) => ({
    ...t,
    net: ok.reduce((acc, m) => acc + (m.stats ? m.stats.flow[t.key].net : 0), 0),
  }));
  // 条形宽度以最大绝对值为基准，让最大的那档占满
  const maxAbs = Math.max(...tierTotals.map((t) => Math.abs(t.net)), 1);

  return (
    <section className="mk-section">
      <h2 className="mk-section-title">
        资金流向
        <span className="mk-section-sub">主力 = 超大单 + 大单</span>
      </h2>

      <div className={`fl-headline fl-${trendClass(mainForceNet)}`}>
        <span className="fl-headline-label">主力净流入</span>
        <span className="fl-headline-value">{formatSignedMoney(mainForceNet)}</span>
      </div>

      <ul className="fl-tiers">
        {tierTotals.map((t) => {
          const cls = trendClass(t.net);
          const w = (Math.abs(t.net) / maxAbs) * 100;
          return (
            <li key={t.key} className="fl-tier">
              <span className="fl-tier-label">{t.label}</span>
              {/* 双向条：净流入向右、净流出向左，中线对齐 */}
              <span className="fl-track">
                <span
                  className={`fl-fill fl-bar-${cls}`}
                  style={{
                    width: `${w / 2}%`,
                    // 流出向左伸：右边缘固定在中线
                    ...(t.net < 0
                      ? { right: "50%" }
                      : { left: "50%" }),
                  }}
                />
                <span className="fl-mid" aria-hidden="true" />
              </span>
              <span className={`fl-tier-value fl-t-${cls}`}>
                {formatSignedMoney(t.net)}
              </span>
            </li>
          );
        })}
      </ul>

      <ul className="fl-markets">
        {markets.map((m) => (
          <li key={m.secid} className="fl-mkt-row">
            <span className="fl-mkt-label">{m.label}</span>
            {m.stats ? (
              <span className="fl-mkt-right">
                <span
                  className={`fl-mkt-value fl-t-${trendClass(
                    m.stats.flow.extra_large.net + m.stats.flow.large.net
                  )}`}
                >
                  {formatSignedMoney(
                    m.stats.flow.extra_large.net + m.stats.flow.large.net
                  )}
                </span>
                {/* 同 BreadthBar：轮转取数下各市场新鲜度不同，必须逐行标注 */}
                {m.fetched_at && (
                  <span className="fl-mkt-time">{m.fetched_at}</span>
                )}
              </span>
            ) : (
              <span className="fl-mkt-err">{m.error ?? "无数据"}</span>
            )}
          </li>
        ))}
      </ul>
    </section>
  );
}
