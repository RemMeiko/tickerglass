# 搜索接口 fixture（Sprint 2A）

**全部真实抓取**，抓取脚本 `harness/capture_fixtures.py`，抓取时间 **2026-08-23**。
低频抓取（每次请求间隔 8~9s）以避免把自己打进限流窗口。

两个源的 URL 模板：

- 主源 `searchapi.eastmoney.com/api/suggest/get?input={q}&type=14&token=...&count=10`
- 兜底 `search-codetable.eastmoney.com/codetable/search/web?client=web&keyword={q}&pageSize=10&pageIndex=1`

> `token=D43BF722C8E33BDC906FB84D85E326E8` 是东财 suggest 接口的**公开固定串**，
> 全网文档与前端 JS 里都是同一个值，不是用户凭证、不含身份信息。
> 它写在 `net.rs` 里属接口地址的一部分，**不是硬编码密钥**。

## 主源 searchapi（裸 JSON，无 JSONP 包裹）

| 文件 | 查询词 | 字节 | 抓来验证什么 | 加工 |
|------|--------|------|-------------|------|
| `searchapi_maotai.json` | `茅台` | 365 | 标准 2 名称搜索 | 无，原样 |
| `searchapi_600519.json` | `600519` | 365 | 标准 2 代码搜索 | 无，原样 |
| `searchapi_gzmt.json` | `GZMT` | 365 | 标准 2 拼音搜索（**不是「mt」**） | 无，原样 |
| `searchapi_aapl.json` | `AAPL` | 3034 | 美股 `MktNum` + 债券噪声 `SecurityType=7` | 无，原样 |
| `searchapi_szzs.json` | `上证指数` | 2915 | **`指数` 白名单必须通过** + 基金噪声 | 无，原样 |
| `searchapi_000001.json` | `000001` | 2400 | `(market, 裸码)` 组合键歧义 + 英股噪声 | 无，原样 |
| `searchapi_tencent.json` | `00700` | 2888 | 港股 `MktNum=116` + 韩股噪声 | 无，原样 |
| `searchapi_empty.json` | `zzzzzzzz` | 106 | **空结果是 `Data: null` 而非 `[]`** | 无，原样 |

### 实测要点（与合同描述的差异，已在代码注释中标注）

1. **两个源当前都返回裸 JSON，没有 JSONP 包裹** —— 合同的「JSONP 剥离」前提与实测不符。
   `strip_jsonp` 仍保留（首字符为 `{` 时原样返回，零误伤），因该接口历史上支持 `&cb=` 回调。
2. **`SecurityTypeName=美股` 挡不住债券**：`Apple Inc Notes 2022` 的 `SecurityTypeName`
   就是「美股」，仅靠类型名白名单会把 8 条债券一起放进自选股。
   实测判据：债券 8/8 均为 `SecurityType=7`，真身 AAPL 是 `20`。
3. `MktNum` / `SecurityType` 在主源是**字符串**（`"1"` / `"105"`），codetable 侧 `market` 是**整数**。

## 兜底源 search-codetable（裸 JSON，结构与主源完全不同）

| 文件 | 查询词 | 字节 | 抓来验证什么 | 加工 |
|------|--------|------|-------------|------|
| `codetable_szzs.json` | `上证指数` | 2824 | `指数` 通过 + 8 条基金噪声被排除 | 无，原样 |
| `codetable_000001.json` | `000001` | 2230 | 同一裸码 3 个市场（深A/指数/基金）+ 英股 | 无，原样 |
| `codetable_h30263.json` | `H30263` | 358 | `market=2` 中证指数（腾讯无身份键） | 无，原样 |
| `codetable_aapl.json` | `AAPL` | 2759 | 债券噪声（`status=20`） | 无，原样 |
| `codetable_00700.json` | `00700` | 2738 | 港股 `market=116` + 基金噪声 | 无，原样 |
| `codetable_empty.json` | `zzzzzzzz` | 112 | 空结果是 `result: []`（**与主源的 `null` 不同**） | 无，原样 |

### 实测要点

1. 顶层 `code` 是状态串 `"0"`，**与条目内证券 `code` 撞名**；`result` 直接是数组，无二层包装。
2. `securityType` 是**整数数组**（`[401,101,5,501,301]`），按标量解析必崩。
3. **codetable 侧不能用「数组含 7」判债券** —— 实测真身 AAPL 的数组同样含 7
   （`[401,501,102,7,201,301]`），照此判定会把正股一起误杀。
   可靠判据是 `status`：真实证券 **31/31 为 10**，债券 **8/8 为 20**。
4. `market=2`（中证指数，如 H30263）**没有对应的腾讯代码前缀**，故这类条目被排除 ——
   加进自选股只会永远显示「—」。详见 self-eval「与合同的偏差」。

## 派生 fixture（唯一一个非原样文件，按合同「允许减法加工但须注明」）

| 文件 | 来历 | 加工方式 |
|------|------|---------|
| `searchapi_rate_limited.json` | 由 `searchapi_maotai.json` **减法**得到 | **删掉 `QuotationCodeTable` 外层键**，保留其内层对象原文 |

减法的依据：限流判据是「有效响应恒含 `QuotationCodeTable` 键，限流响应体完全没有」。
该 fixture 用于单测「缺键 → 判 `RateLimited`」这一条判定分支，**数值全部来自真实响应，未编造**。

真实限流响应体的实打观察见 self-eval「限流实打测试」一节
（按 Evaluator N-5 要求排在标准 2 正常搜索验收**之后**执行）。
