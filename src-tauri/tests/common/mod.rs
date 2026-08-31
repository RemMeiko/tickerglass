//! fixture 读取辅助。**不发起任何网络请求** —— 只读 tests/fixtures/ 下的文件。

use std::path::PathBuf;

pub fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

/// 报价 fixture 以 GBK **字节**存盘，故按字节读入，
/// 让测试真正走 `parse::gbk` 的解码路径（存成 UTF-8 就测不到这条链路了）。
pub fn read_bytes(name: &str) -> Vec<u8> {
    std::fs::read(fixture_path(name))
        .unwrap_or_else(|e| panic!("读取 fixture {name} 失败: {e}"))
}

/// JSON / HTML fixture 按 UTF-8 文本读入。
pub fn read_text(name: &str) -> String {
    String::from_utf8_lossy(&read_bytes(name)).into_owned()
}
