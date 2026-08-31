// 防止 Windows release 构建弹出多余的控制台窗口
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    tickerglass_lib::run()
}
