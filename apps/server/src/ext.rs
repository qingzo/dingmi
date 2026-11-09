//! 扩展点（需求-1.0 第 32.1 节第 12 条「分级开源」）：开源的 dingmi-server 只管「自己搭、自己同步」，
//! 叮咪同步（兑换码、套餐、到期、公测、邮箱、客服核对码、连接日志、告警、外部备份）不在这里。
//!
//! 同步服务器在下面这些地方问一句 [`Ext`]，开源版用 [`SelfHosted`]：什么都不做，叮咪同步才有的消息回「这台服务器不支持」
//! （和以前自建服务器没开兑换码时的回法一样，电脑端认得）。官方版在编译时换成自己的实现（`Server::load_with`），
//! 不开端口、不动态加载任何东西，`config` 也改不了这里用哪一个。
//!
//! 规矩：
//! - 除了 [`Ext::open_space`]（在握手里，和以前 Hello.redeem / beta 的位置一样）和 [`Ext::open`]（启动时建表），
//!   别的都在 Noise 握手做完、认出是哪台设备（或者手机令牌对上）之后才调。握手和加密不经过这里，这里拿不到也用不着任何密钥。
//! - 实现方查不到、出错时一律**拒绝**（比如查不到到期日就当到期了），不放行。

use crate::net::Server;
use crate::store::Store;
use dingmi_sync::{refuse, Account, BetaInfo, Hello, Msg, Refusal};
use std::net::IpAddr;
use std::sync::Arc;

/// 这台服务器不支持（自建服务器）时回的话
pub const REDEEM_OFF_MSG: &str = "这台服务器没开兑换码：兑换码只能在叮咪同步里用";
pub const BETA_OFF_MSG: &str = "这台服务器没有公测免费体验：公测只在叮咪同步里有";
pub const EMAIL_OFF_MSG: &str = "只有叮咪同步能留邮箱";
pub const SUPPORT_OFF_MSG: &str = "这台服务器没有客服核对码：自己搭的服务器有问题，找服务器的管理员";
pub const JOIN_OFF_MSG: &str = "这台服务器没有兑换码加入：自己搭的服务器用配对码、手机或恢复码加入";
pub const DELETE_OFF_MSG: &str = "自己搭的服务器在服务器上删：停用这个空间（dingmi-server disable），满期连数据一起删掉";

/// 数据库 kv 里记着这份数据是哪个版本的程序在用：官方版启动时（它的 [`Ext::open`]）写上 [`OFFICIAL`]。
/// 开源版看到这个标记就不启动（默认的 [`Ext::open`]），免得把开源版误放到官方服务器上：开源版不认到期、
/// 不删、不记日志，到期的空间会接着免费同步。确定要拿开源版接着用，先执行 `dingmi-server edition --self-hosted --yes` 去掉标记
pub const EDITION_KEY: &str = "edition";
pub const OFFICIAL: &str = "official";
pub const OFFICIAL_DB_MSG: &str = "这是叮咪同步官方版的数据库，要用官方版程序。\
确定要拿开源版接着用（叮咪同步的兑换码、到期、设备上限、连接日志就都不管了），先执行 dingmi-server edition --self-hosted --yes";

/// 没登记过的设备想开通新空间（[`Ext::open_space`]）的结果
pub enum Opening {
    /// 开通了：新空间的编号，接着照常登记这台电脑（停用、[`Ext::stopped`]、[`Ext::room`] 照样要过）
    Space(String),
    /// 只是问问公测（Hello.ask_beta）：回 Welcome.beta 后断开，不登记、不算违规
    Beta(BetaInfo),
    /// 兑换码加入的第一步（Hello.join_ask）：回 Welcome.join_salt 后断开，不登记、不算违规
    JoinSalt(String),
    /// 说得清的拒绝：协议 5 起的电脑收到 Welcome.refused，旧电脑直接断开；不算违规
    Refused(Refusal),
    /// 和陌生设备一样：什么也不回、直接断开，算一次违规
    Unknown,
}

/// 单个附件（图片、文件）的限额（叮咪同步按套餐，32.1 第 5 条）
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FileLimits {
    /// 给电脑和手机网页看的（`/m/api/limits`）：单个图片、单个附件最大多大，None 不限
    pub max_image: Option<u64>,
    pub max_file: Option<u64>,
    /// 服务器拦的上限（密文的字节数），None 不拦（只有 60 MB 的总上限）。查不到时实现方给 Some(0)：什么附件都不收
    pub cap: Option<u64>,
    /// 超了时给人看的话
    pub too_big: String,
}

/// 一条消息是谁发来的（[`Ext::handle`]）
pub struct Who<'a> {
    pub space: &'a str,
    /// 电脑的公钥
    pub device: &'a str,
    pub ip: IpAddr,
}

/// 扩展点。每个方法写了什么时候调、开源版（默认）做什么
pub trait Ext: Send + Sync + 'static {
    /// 启动时（`Server::load_with`）、命令行打开数据库时（`cli::opened`）：建自己的表。出错就不启动。
    /// 默认：数据库带着官方版的标记（[`EDITION_KEY`]）就不启动，别的什么都不做
    fn open(&self, store: &Store) -> Result<(), String> {
        if store.kv_get(EDITION_KEY).as_deref() == Some(OFFICIAL) {
            return Err(OFFICIAL_DB_MSG.into());
        }
        Ok(())
    }

    /// `version` 里显示的：开源版还是官方版
    fn edition(&self) -> &'static str {
        "开源版"
    }

    /// 每天收尾时删停用满期的空间（`admin::purge_due_with`）之前问一句：false 就先不删（比如官方版冻结了的空间），明天再问。
    /// 默认：true
    fn may_purge(&self, _store: &Store, _space: &str) -> bool {
        true
    }

    /// 握手里：没登记过的设备既没带配对码、也没带恢复凭证（或者只是问公测，Hello.ask_beta；
    /// 协议 7 起还有兑换码加入：Hello.join_ask 回 [`Opening::JoinSalt`]，Hello.join 核对上了回 [`Opening::Space`]）。
    /// 默认：问公测回「没开」；带兑换码（开通、加入）回 redeem_off、要公测回 beta_off（说清楚、不算违规）；别的照陌生设备断开
    fn open_space(&self, _s: &Server, hello: &Hello, _device: &str, _ip: IpAddr) -> Opening {
        if hello.ask_beta {
            Opening::Beta(BetaInfo::default())
        } else if hello.join_ask.is_some() || hello.join.is_some() {
            Opening::Refused(Refusal { code: refuse::REDEEM_OFF.into(), msg: JOIN_OFF_MSG.into() })
        } else if hello.redeem.is_some() {
            Opening::Refused(Refusal { code: refuse::REDEEM_OFF.into(), msg: REDEEM_OFF_MSG.into() })
        } else if hello.beta {
            Opening::Refused(Refusal { code: refuse::BETA_OFF.into(), msg: BETA_OFF_MSG.into() })
        } else {
            Opening::Unknown
        }
    }

    /// 收不收电脑传上来的盒子 3 和加入核对值（兑换码加入，需求-1.x 第 34.2 节）。
    /// 默认：false。自建服务器用不了兑换码加入，存着盒子 3 只会让偷到数据库的人能离线猜同步密码：
    /// SetupSpace、SetLock 带来的不存，JoinSet 说不支持，启动时清掉以前（1.0.6、1.0.7 的电脑）传上来的
    fn joins(&self) -> bool {
        false
    }

    /// 空间被停了（叮咪同步到期）：电脑的每条消息之前（Ping、AccountGet 和交给 [`Ext::handle`] 的那几条除外）、
    /// 手机网页的每个请求之前（回 403 和这句话）、登记新设备之前。Some 就拒绝。
    /// 默认：None（自建服务器不会到期）
    fn stopped(&self, _s: &Server, _space: &str) -> Option<Refusal> {
        None
    }

    /// 还能不能再加一台设备：登记新电脑、电脑添加手机之前。默认：Ok（自建服务器不限台数）
    fn room(&self, _s: &Server, _space: &str) -> Result<(), Refusal> {
        Ok(())
    }

    /// 给电脑的账（Welcome.account、AccountGet）。`base` 是服务器自己算的：用了多少、容量上限、满没满、几台设备。
    /// 默认：原样回 `base`（official 是 false，电脑不显示套餐、到期、续费）
    fn account(&self, _s: &Server, _space: &str, base: Account) -> Account {
        base
    }

    /// 叮咪同步才有的消息：Renew、SupportNew、EmailSet、EmailDel、SpaceDelete。Err 回 Refused（旧电脑是 Error）。
    /// 默认：都回「这台服务器不支持」（redeem_off）
    fn handle(&self, _s: &Server, _who: &Who, msg: Msg) -> Result<Msg, Refusal> {
        let msg = match msg {
            Msg::EmailSet { .. } | Msg::EmailDel => EMAIL_OFF_MSG,
            Msg::SupportNew => SUPPORT_OFF_MSG,
            Msg::SpaceDelete { .. } => DELETE_OFF_MSG,
            _ => REDEEM_OFF_MSG,
        };
        Err(Refusal { code: refuse::REDEEM_OFF.into(), msg: msg.into() })
    }

    /// 单个附件的限额：电脑上传附件（FilePut）、手机网页上传附件、手机网页问限额（`/m/api/limits`）时。
    /// 默认：不限（只有 60 MB 的总上限）
    fn file_limits(&self, _s: &Server, _space: &str) -> FileLimits {
        FileLimits::default()
    }

    /// 发生了一件该记下来的事（连接日志）：设备连上、配对、被拒、添加手机、作废另一台电脑、手机连上……
    /// `what` 是给人看的说法。没登记的设备被拒时 `space` 是 None（这时 Noise 握手已经做完，公钥是对方证明过的）。
    /// 默认：什么都不记（服务器的输出里照旧有 log 那几行）
    fn event(&self, _s: &Server, _ip: IpAddr, _space: Option<&str>, _device: Option<&str>, _what: &str) {}

    /// 回给电脑一条 Error（服务器这边出错，不是按规矩拒绝）。默认：什么都不做
    fn error(&self) {}

    /// 每天收尾做完以后（本地备份、清过期的配对码、删停用满期的空间、换证书之后）。`backup` 是今天本地备份的结果。
    /// 默认：什么都不做
    fn daily(&self, _s: &Arc<Server>, _backup: Result<(), String>) {}

    /// 空间满了时怎么办（接在「删掉一些附件、清空回收站，」后面）
    fn full_hint(&self, _s: &Server) -> &'static str {
        "或者请服务器管理员调高上限"
    }
}

/// 开源版：自己搭、自己同步，别的都没有
#[derive(Default)]
pub struct SelfHosted;

impl Ext for SelfHosted {}

/// 去掉官方版的标记（`dingmi-server edition --self-hosted --yes`）：以后开源版能打开这份数据。返回原来有没有
pub fn clear_edition(store: &Store) -> Result<bool, String> {
    let n = store.c().execute("DELETE FROM kv WHERE key=?1", [EDITION_KEY]).map_err(|e| e.to_string())?;
    Ok(n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{admin, cli};

    /// 官方版用过的数据库（带着标记）：开源版不启动、命令行也打不开；去掉标记以后照常。
    /// 没有标记的（老的自建库，带着叮咪同步那些列也一样，见 net.rs 的 official_tables_are_ignored）照常
    #[test]
    fn official_db_refuses_open_server() {
        let dir = std::env::temp_dir().join(format!("dingmi-edition-{}", rand::random::<u64>()));
        crate::init(&dir, "127.0.0.1".parse().unwrap(), 7788).unwrap();
        assert!(Server::load(&dir).is_ok(), "没有标记照常");
        assert!(cli::opened(&dir).is_ok());
        Store::open(&dir).unwrap().kv_set(EDITION_KEY, OFFICIAL).unwrap();
        let e = Server::load(&dir).err().unwrap();
        assert!(e.contains("官方版的数据库") && e.contains("edition --self-hosted --yes"), "{e}");
        assert!(cli::opened(&dir).err().unwrap().to_string().contains("官方版的数据库"));
        assert!(clear_edition(&Store::open(&dir).unwrap()).unwrap());
        assert!(Server::load(&dir).is_ok());
        assert!(!clear_edition(&Store::open(&dir).unwrap()).unwrap(), "去过了就没有了");
        assert_eq!(SelfHosted.edition(), "开源版");
    }

    /// 停用满期删空间前问扩展点：说不删的留着
    #[test]
    fn purge_asks_ext() {
        let dir = std::env::temp_dir().join(format!("dingmi-purge-{}", rand::random::<u64>()));
        crate::init(&dir, "127.0.0.1".parse().unwrap(), 7788).unwrap();
        let s = Store::open(&dir).unwrap();
        let a = admin::invite(&s, None).unwrap().space;
        let b = admin::invite(&s, None).unwrap().space;
        s.set_disabled(&a, Some("2000-01-01 00:00:00"));
        s.set_disabled(&b, Some("2000-01-01 00:00:00"));
        let done = admin::purge_due_with(&s, &dir, |sp| sp != a);
        assert_eq!(done.into_iter().map(|d| d.0).collect::<Vec<_>>(), vec![b.clone()]);
        assert!(admin::spaces(&s).iter().any(|x| x.id == a), "说了不删的还在");
        assert!(SelfHosted.may_purge(&s, &a));
    }
}
