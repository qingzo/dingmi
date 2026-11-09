//! 密钥放在 Windows 凭据管理器里，不写进数据库文件：数据库被复制走也拿不到它们。

use keyring::Entry;

/// 调试时用单独的数据目录（DINGMI_DATA_DIR）运行，密钥也分开放，不碰正在用的叮咪的
fn service() -> &'static str {
    if crate::dev_data_dir().is_some() {
        "dingmi-dev"
    } else {
        "dingmi"
    }
}

/// 这台电脑的 Noise 私钥，服务器靠它认出这台电脑
pub const DEVICE_KEY: &str = "sync-device-key";
/// 主密钥：用同步密码从服务器上的盒子 1 里取出来的
pub const DATA_KEY: &str = "sync-data-key";
/// 空间密钥：和同步密码一起锁盒子 1，只在设备上
pub const SPACE_KEY: &str = "sync-space-key";
/// 备份的主密钥：和同步的分开，换同步方式、停止同步都不影响备份
pub const BACKUP_KEY: &str = "sync-backup-key";

pub fn set(name: &str, secret: &[u8]) -> Result<(), String> {
    Entry::new(service(), name)
        .and_then(|e| e.set_secret(secret))
        .map_err(|e| format!("保存密钥到 Windows 凭据管理器失败：{e}"))
}

/// 读一把钥匙：`Ok(None)` 是真的没有（重装过系统、从别的电脑拷来的数据）；`Err` 是这次没读出来
/// （凭据管理器出错），过一会儿再试，不能当成没有（需求-1.x 第 34 节开工前的检查）
pub fn read(name: &str) -> Result<Option<Vec<u8>>, String> {
    match Entry::new(service(), name).and_then(|e| e.get_secret()) {
        Ok(v) => Ok(Some(v)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("读 Windows 凭据管理器出错：{e}")),
    }
}

/// 读不出来也当没有，用在出错了只是这一次做不成的地方；出错的原因记进日志（同一个原因只记一次）
pub fn get(name: &str) -> Option<Vec<u8>> {
    read(name).unwrap_or_else(|e| {
        static LAST: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());
        let mut last = LAST.lock().unwrap();
        if *last != e {
            crate::diag::log("同步", format!("{name}：{e}"));
            *last = e;
        }
        None
    })
}

pub fn delete(name: &str) {
    if let Ok(e) = Entry::new(service(), name) {
        let _ = e.delete_credential();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 会真的写一条到 Windows 凭据管理器再删掉，平时跳过：cargo test --lib -- --ignored credential_roundtrip
    #[test]
    #[ignore]
    fn credential_roundtrip() {
        let name = "sync-test-only";
        set(name, &[1, 2, 3, 255]).unwrap();
        assert_eq!(get(name), Some(vec![1, 2, 3, 255]));
        delete(name);
        assert_eq!(get(name), None);
    }
}
