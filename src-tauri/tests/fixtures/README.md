# 测试 fixture 来源与加工说明

全部 fixture 于 **2026-08-21 23:27–23:32（北京时间）** 用 `curl` 真实抓取（请求头带
spec 指定的 UA + Referer）。当时 A 股/港股已收盘（当日完整分时），美股正在盘中。

抓取地址：
- 报价 `https://qt.gtimg.cn/q={codes}`
- 腾讯分时 `https://web.ifzq.gtimg.cn/appstock/app/minute/query?code={code}`
- 东财分时 `https://push2delay.eastmoney.com/api/qt/stock/trends2/get?secid={secid}&ndays=1&fields1=f1,f2,f3,f4,f5&fields2=f51,f52,f53,f54,f55,f56,f57,f58`

**加工规则**（合同硬约束）：只允许两类 —— ① 从真实响应中**截取**片段（不改任何字符）；
② 对真实报文做**减法**（截断、删字段、置空）。**任何数值都不是编造的**，
没有任何 fixture 含加法造数。下表逐个注明。

## 报价类（`.txt`，GBK 字节原样存盘）

| 文件 | 加工方式 | 来源 |
|------|---------|------|
| `quote_ashare.txt` | **原样**（零加工） | `q=sh600519` |
| `quote_index.txt` | **原样** | `q=sh000001` |
| `quote_hk.txt` | **原样** | `q=hk00700` |
| `quote_us.txt` | **原样** | `q=usAAPL` |
| `quote_us_suffix.txt` | **原样**（与 `quote_us.txt` 同一份抓取，`[2]` 天然为 `AAPL.OQ`） | `q=usAAPL` |
| `quote_mixed_4.txt` | **原样**（一次请求四个代码的真实四行响应） | `q=sh000001,sh600519,hk00700,usAAPL` |
| `quote_missing_line.txt` | **原样** —— 请求了 2 个代码，响应里 `sh999999` 整行**真的不存在**，只回 `v_sz000001` 一行。这份 fixture 的价值就在于它是原样的 | `q=sh999999,sz000001` |
| `quote_ts_formats.txt` | **截取**：从 `quote_mixed_4.txt` 中取出承载三种时间戳格式的 3 行（指数 `20260821161402` / 港股 `2026/08/21 16:08:14` / 美股 `2026-08-21 11:27:40`），未改任何字符 | 同上 |
| `quote_none_match.txt` | **原样** —— 单个无效代码的真实响应是 `v_pv_none_match="1";`（不是 `v_{code}="";`） | `q=sz999999` |
| `quote_empty.txt` | **减法**：取 `quote_ashare.txt`，把 `="…"` 的引号内容清空 → `v_sh600519="";`（构造 spec 描述的空串形态） | 同 `quote_ashare.txt` |
| `quote_short_fields.txt` | **减法**：取 `quote_ashare.txt`，只保留前 3 个 `~` 字段，其余删除 | 同上 |
| `quote_nonnumeric.txt` | **减法**：取 `quote_ashare.txt`，把现价字段 `[3]` **置空**（其余字段一字未动） | 同上 |
| `quote_truncated.txt` | **减法**：取 `quote_ashare.txt`，从中间截断（丢掉行尾 `";`） | 同上 |
| `quote_mixed_one_broken.txt` | **减法**：取 `quote_mixed_4.txt`，把第 2 行（`sh600519`）截断到约 1/3 长度，另外 3 行原样 | 同 `quote_mixed_4.txt` |

## 分时类

| 文件 | 加工方式 | 来源 / 实测特征 |
|------|---------|----------------|
| `minute_normal.json` | **原样** | `code=sh000001`，242 点，`0930 → 1500` |
| `minute_after_close.json` | **原样** | `code=sh600519`，**267 点**，`0930 → 1530`，含 15:06–15:30 的 25 个盘后重复点 |
| `minute_hk_lunch.json` | **原样** | `code=hk00700`，**332 点**，`0930 → 1608`，午休在 `1200 → 1300`，末尾含盘后 `1601–1608` |
| `minute_with_lunch.json` | **截取**：从 `minute_after_close.json` 的真实点数组中取 `1128 ≤ hhmm ≤ 1302` 的片段（实际为 6 点：`1128 1129 1130 1300 1301 1302`），JSON 其余部分原样。**未手编任何时间序列** | 同 `sh600519` |
| `minute_empty_data.json` | **减法**：取 `minute_normal.json`，把点数组置为 `[]`、`date` 置空（构造盘前形态），`qt` 节点原样保留（昨收仍可取） | 同 `sh000001` |
| `minute_zero_price.json` | **减法**：取 `minute_normal.json`，把第 11 个点的价格 token 改为 `0.00`、第 12 个点的价格 token **置空**。仅这两个 token 被减法处理，其余点原样。（说明：实测遍历 8 个不同标的均未发现天然的 0 价点，故用减法构造；未编造任何新数值） | 同 `sh000001` |
| `minute_missing_key.json` | **原样** —— 无效代码的真实响应 `{"code":-1,"msg":"code param error","data":[]}`，`data` 是数组而非对象，故取不到 `data.{code}` | `code=zz123456` |
| `minute_html_error.html` | **原样** —— 真实 HTML 错误页（带 UTF-8 BOM + `<!DOCTYPE html>`），用于 `ParseError::NotJson` 路径 | `https://quote.eastmoney.com/nonexistent-page-xyz.html`（404 HTML 页） |
| `em_trends_us.json` | **原样** | `secid=105.AAPL`，**120 点**，时间戳是**北京时间**：首点 `2026-08-21 21:30`（= 美东 09:30 EDT 开盘），末点 `2026-08-21 23:29` |
| `em_trends_winter.json` | **减法/平移**：取 `em_trends_us.json`，只改**时间字段**，**日期与时刻一起平移** 到 `2026-12-15`（EST 冬令时）—— `21:30 → 22:30`（+1h），跨零点的行落到 `2026-12-16`。**价格/成交量等数值一字未动**。为什么要一起平移：冬令时京美时差是 13h，美股 09:30 开盘对应北京 **22:30**（不是夏令时的 21:30）；若只改日期不改时刻，首点会算出 `hhmm=830` 落在 391 格美东栅格外被丢弃 | 同上 |
| `em_trends_null.json` | **原样** —— 无效 secid 的真实响应，`data` 为 `null` | `secid=bogus` |

## 硬约束

- 任何 `tests/*.rs` **不发起网络请求**，只读本目录文件。
- 报价 fixture 以 **GBK 字节**存盘，测试从字节流读入，真正走 `parse::gbk` 解码路径
  （若存成 UTF-8，GBK 解码这条链路就测不到了）。
