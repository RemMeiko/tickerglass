import type { Market, Quote } from "../types";

/** 数据未到达 / 不可得时的占位符。绝不用 0 冒充 */
export const PLACEHOLDER = "—";

/**
 * 内部身份键（`tencent_code`）→ 界面显示用的证券代码。
 *
 * 内部键带市场前缀（`sh000001`/`sz000001`/`hk00700`/`usAAPL`），这是**必须的** ——
 * 东财裸码不唯一（`000001` 在 market=0 是平安银行、market=1 是上证指数），
 * 所以身份键不能剥前缀。但**显示**给用户时应与行情软件一致，只给裸码。
 *
 * 前缀规则：A股/指数 `sh`/`sz`（两位小写 + 6 位数字）、港股 `hk` + 5 位数字、
 * 美股 `us` + 字母代码。不匹配已知前缀时原样返回（宁可多显示前缀，不要错剥）。
 */
export function displayCode(tencentCode: string): string {
  const m = /^(?:sh|sz|hk|us)(.+)$/i.exec(tencentCode);
  return m ? m[1] : tencentCode;
}

/** 按市场的默认小数位。港股实测 3 位（457.000），A 股/指数/美股 2 位 */
function defaultDecimals(market: Market): number {
  return market === "hk" ? 3 : 2;
}

/**
 * 价格格式化。**优先保留报文原始小数位数** ——
 * 港股统一按 2 位会显示成 457.00，与东财网页不一致，会被误判为解析 bug。
 */
export function formatPrice(
  price: number | null | undefined,
  market: Market,
  reportDecimals?: number | null
): string {
  if (price === null || price === undefined || !Number.isFinite(price)) {
    return PLACEHOLDER;
  }
  const d = reportDecimals ?? defaultDecimals(market);
  return price.toFixed(d);
}

/** 涨跌幅统一 2 位 + %，带符号 */
export function formatPercent(percent: number | null | undefined): string {
  if (percent === null || percent === undefined || !Number.isFinite(percent)) {
    return PLACEHOLDER;
  }
  const sign = percent > 0 ? "+" : "";
  return `${sign}${percent.toFixed(2)}%`;
}

/** 涨跌额，小数位跟随价格 */
export function formatAmount(
  amount: number | null | undefined,
  market: Market,
  reportDecimals?: number | null
): string {
  if (amount === null || amount === undefined || !Number.isFinite(amount)) {
    return PLACEHOLDER;
  }
  const d = reportDecimals ?? defaultDecimals(market);
  const sign = amount > 0 ? "+" : "";
  return `${sign}${amount.toFixed(d)}`;
}

export function quotePrice(quote: Quote | null, market: Market): string {
  if (!quote) return PLACEHOLDER;
  return formatPrice(quote.price, market, quote.price_decimals);
}
