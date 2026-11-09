//! dingmi-server：叮咪的同步服务器。
//!
//! 只开一个端口（默认 7788）：叮咪电脑端用 Noise 加密连接，手机网页用 HTTPS（自签根证书）。
//! 服务器只保存密文，看不到提醒和笔记的内容。部署方法见 docs/自建服务器.md。
//! 代码都在库里（lib.rs），命令行在 cli.rs。

fn main() -> std::process::ExitCode {
    dingmi_server::cli::main(None)
}
