/// GBK → UTF-8 解码。
///
/// 腾讯报价接口返回 GBK 编码，必须按**字节**读入后在这里解码。
/// 绝不能用 `reqwest` 的 `.text()`（它按 UTF-8 猜解码，股票中文名会全部变乱码）。
/// 用 `GBK` 解码器（覆盖 GB18030 超集，兼容性优于 GB2312）；
/// 非法字节走替换字符而不是 panic。
pub fn decode_gbk(bytes: &[u8]) -> String {
    let (text, _, _) = encoding_rs::GBK.decode(bytes);
    text.into_owned()
}
