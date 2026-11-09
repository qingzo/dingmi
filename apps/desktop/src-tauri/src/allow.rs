//! 界面拿路径来叫程序打开、选中、读、写时先核对：只认这次用户自己在对话框里选过的、拖进来的，
//! 启动器里的（自己加的文件夹里的文件也算），以及叮咪自己的几个地方（数据文件夹、附件、同步和备份文件夹、诊断包）。
//! 网页万一被塞进了别的脚本（XSS），也不能拿 open_path 去运行随便哪个程序、拿 restore_apply 读随便哪个文件。
//!
//! 只认本机的盘符路径（C:\…）；`\\服务器\共享`、`\\?\`、`\\.\` 这类除非是用户自己选的，一律不认
//! （打开网络路径时 Windows 会自动把登录凭据发过去）。

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf, Prefix};
use std::sync::{LazyLock, Mutex};

/// 这次运行里用户选过、拖进来过的路径（规整过的写法）
static GRANTED: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));
/// 拖进来的快捷方式解开后的 (目标, 参数)：启动器「自己加的」只有这样来的才能带参数
static GRANTED_ARGS: LazyLock<Mutex<HashMap<String, String>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// 规整成好比较的写法：/ 换成 \，去掉末尾的 \，Windows 上不分大小写
pub fn norm(p: &str) -> String {
    let s = p.trim().replace('/', "\\");
    let s = s.trim_end_matches('\\');
    // 「C:\」去掉末尾的 \ 以后是「C:」，补回来
    let s = if s.len() == 2 && s.ends_with(':') { format!("{s}\\") } else { s.to_string() };
    if cfg!(windows) { s.to_lowercase() } else { s }
}

/// 用户自己选的、拖进来的：记下来，之后这个路径（和它下面一层的文件）能打开
pub fn grant(p: &str) {
    if !p.trim().is_empty() {
        GRANTED.lock().unwrap().insert(norm(p));
    }
}

pub fn granted(p: &str) -> bool {
    GRANTED.lock().unwrap().contains(&norm(p))
}

/// 拖进来的快捷方式解开成的一项：目标和参数一起记下
pub fn grant_custom(target: &str, args: &str) {
    grant(target);
    GRANTED_ARGS.lock().unwrap().insert(norm(target), args.to_string());
}

/// 这个目标带这些参数是拖进来的快捷方式里本来就有的吗
pub fn granted_args(target: &str, args: &str) -> bool {
    args.trim().is_empty() || GRANTED_ARGS.lock().unwrap().get(&norm(target)).is_some_and(|a| a == args)
}

/// 本机盘符开头的绝对路径，没有 ..（不是网络路径、设备路径）
pub fn local_abs(p: &str) -> bool {
    let path = Path::new(p.trim());
    let mut comps = path.components();
    let disk = matches!(comps.next(), Some(Component::Prefix(x)) if matches!(x.kind(), Prefix::Disk(_)));
    disk && matches!(comps.next(), Some(Component::RootDir)) && !comps.any(|c| matches!(c, Component::ParentDir))
        // 备用数据流（a.txt:stream）、设备名不认；盘符后面的冒号已经在 Prefix 里了
        && !p.trim().get(2..).unwrap_or("").contains(':')
}

/// `p` 是 `dir` 本身或者在它下面（都先规整过）
pub fn under(p: &str, dir: &Path) -> bool {
    let (p, d) = (norm(p), norm(&dir.display().to_string()));
    !d.is_empty() && (p == d || p.strip_prefix(&d).is_some_and(|rest| rest.starts_with('\\')))
}

/// `p` 正好在 `dir` 下面一层（文件夹里的文件）
pub fn child_of(p: &str, dir: &str) -> bool {
    let p = PathBuf::from(norm(p));
    p.parent().is_some_and(|parent| parent.as_os_str() == norm(dir).as_str())
}

/// 用户选过的、拖进来的，或者在用户选过的文件夹下面一层
pub fn granted_or_child(p: &str) -> bool {
    if granted(p) {
        return true;
    }
    let Some(parent) = PathBuf::from(norm(p)).parent().map(|x| x.display().to_string()) else { return false };
    local_abs(p) && granted(&parent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_paths_only() {
        assert!(local_abs(r"C:\Users\Public\Desktop\x.txt"));
        assert!(local_abs("D:/data/x"));
        for bad in [
            r"\\evil.example\share\x.exe",
            r"\\?\C:\Windows\System32\cmd.exe",
            r"\\.\PhysicalDrive0",
            r"C:\a\..\Windows\System32\cmd.exe",
            r"C:relative\x",
            r"relative\x",
            r"C:\a\b.txt:hidden",
            "https://example.com/a.exe",
            "",
        ] {
            assert!(!local_abs(bad), "{bad}");
        }
    }

    #[test]
    fn under_and_child() {
        let dir = Path::new(r"C:\Users\Public\AppData\Roaming\app.dingmi");
        assert!(under(r"c:\users\public\appdata\roaming\app.dingmi\backups", dir));
        assert!(under(r"C:\Users\Public\AppData\Roaming\app.dingmi", dir));
        // 前缀一样但不是它下面
        assert!(!under(r"C:\Users\Public\AppData\Roaming\app.dingmi-evil\x", dir));
        assert!(child_of(r"D:\pics\a.png", r"D:\pics\"));
        assert!(!child_of(r"D:\pics\sub\a.png", r"D:\pics"));
    }

    #[test]
    fn grants() {
        assert!(!granted(r"E:\grant-test\picked"));
        grant(r"E:\grant-test\Picked\");
        assert!(granted(r"e:\grant-test\picked"));
        assert!(granted_or_child(r"E:\grant-test\picked\a.exe"));
        assert!(!granted_or_child(r"E:\grant-test\picked\sub\a.exe"));
        assert!(!granted_or_child(r"E:\grant-test\other.exe"));
        // 参数只认拖进来的快捷方式里本来就有的
        assert!(granted_args(r"E:\grant-test\app.exe", ""));
        assert!(!granted_args(r"E:\grant-test\app.exe", "/c evil"));
        grant_custom(r"E:\grant-test\app.exe", "--profile 2");
        assert!(granted_args(r"E:\grant-test\APP.exe", "--profile 2"));
        assert!(!granted_args(r"E:\grant-test\app.exe", "/c evil"));
    }
}
