// 发布版不弹出命令行窗口
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    dingmi_lib::run()
}
