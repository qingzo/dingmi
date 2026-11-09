//! 叮咪电脑端和服务端共用的同步协议。
//!
//! 连接方式：TCP 连上后，电脑端先发 4 个字节 `DNGM`，再做 Noise IK 握手（和 WireGuard 同一类握手）。
//! 电脑端提前知道服务器的公钥（来自配对码），服务器只认登记过的设备公钥；
//! 第一次连接的新设备在握手消息里带上一次性邀请码，服务器核对后登记它。
//! 握手之后，双方用 JSON 消息（见 [`Msg`]）通信，每条消息都经过 Noise 加密。

mod conn;
mod pair;
pub mod ws;

pub use conn::{accept, connect, generate_keypair, NoiseConn, Pending};
pub use pair::PairCode;

use serde::{Deserialize, Serialize};
use std::fmt;

/// 电脑端连接的开头，服务器靠它和浏览器的 HTTPS / HTTP 连接区分开。
pub const MAGIC: &[u8; 4] = b"DNGM";
pub const NOISE_PARAMS: &str = "Noise_IK_25519_ChaChaPoly_BLAKE2s";
/// 协议版本。1：配对、手机收件箱；2：加上电脑之间的记录同步（Wait 改回 Changed）；3：加上电脑端的新版本（Update*，0.1 起去掉，改从官网检查更新）；
/// 4：叮咪 0.1，主密钥和盒子、恢复码（docs/方案/同步方案.md 第 5.1 节）；
/// 5：叮咪 1.0，叮咪同步：兑换码开通（Hello.redeem）、续费、空间的账（Account）、客服核对码、
/// 说得清的拒绝原因（Refused、Welcome.refused），满了只拦附件（同步方案.md 第 16 节）；
/// 6：叮咪 1.0，套餐带限额（设备数、单个图片 / 附件多大，Account.max_image / max_file）、公测免费体验（Hello.beta、
/// Hello.ask_beta → Welcome.beta）、邮箱（Hello.email、EmailSet / EmailDel、Account.email）、开通时同意的协议版本
/// （Hello.terms、Renew.terms），见同步方案.md 第 16.9 节；
/// 7：叮咪 1.x，新电脑方便加入（需求-1.x 第 34 节）：兑换码 + 同步密码加入（Hello.join_ask → Welcome.join_salt、Hello.join，
/// 盒子 3 和加入核对值：Space.box3、SetupSpace / SetLock 带上、JoinSet / JoinOff、Account.join），用手机加入（Hello.phone_ask、
/// Hello.phone_poll → Welcome.phone），删掉服务器上的数据（SpaceDelete，第 35 节）
pub const VERSION: u32 = 7;
/// 下载手机发来的内容、上传下载图片时，每块多大
pub const INBOX_CHUNK: usize = 256 * 1024;

/// 握手第一条消息里附带的内容（已经用服务器公钥加密）
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(default)]
pub struct Hello {
    pub version: u32,
    /// 设备名，比如"公司电脑"
    pub name: String,
    /// 新设备第一次连接时带上配对码里的邀请码
    pub invite: Option<String>,
    /// 没有邀请码时，用恢复码派生的恢复凭证登记新设备（官方同步没有命令行可以生成配对码）
    pub recovery: Option<String>,
    /// 协议 5 起：没有邀请码、也没有恢复凭证时，用叮咪同步的兑换码（`DM-XXXX-XXXX-XXXX-XXXX`）开通一个新空间，
    /// 登记这台电脑（需求-1.0 第 31.3 节）
    pub redeem: Option<String>,
    /// 协议 6 起：没有邀请码、恢复凭证、兑换码时，领一个公测免费体验的空间（服务器开着公测、还有名额才成），登记这台电脑
    pub beta: bool,
    /// 协议 6 起：只问问公测开没开、还剩几个名额（开通前在「设置 → 同步」里显示）。服务器回 Welcome.beta 后就断开，
    /// 不登记、不算违规
    pub ask_beta: bool,
    /// 协议 6 起：开通（兑换码、公测）时同意的协议版本，比如 "2026-11-07"。叮咪同步的服务器开通时必须带
    pub terms: Option<String>,
    /// 协议 6 起：开通时选填的邮箱（只用来发服务调整、到期和活动通知）
    pub email: Option<String>,
    /// 协议 7 起：兑换码加入的第一步，带兑换码问这个空间的 salt（Welcome.join_salt），回完就断开、不登记
    pub join_ask: Option<String>,
    /// 协议 7 起：兑换码加入：兑换码找到空间，核对值对上就登记这台电脑（Welcome.space 里带盒子 3）
    pub join: Option<Join>,
    /// 协议 7 起：用手机加入的第一步：带这台电脑的临时公钥，服务器回 8 位数字（Welcome.phone），回完就断开
    pub phone_ask: Option<PhoneAsk>,
    /// 协议 7 起：等手机允许：最多等 20 秒，手机允许了 Welcome.phone.sealed 里是加密好的配对信息，回完就断开
    pub phone_poll: Option<PhoneAsk>,
}

/// 兑换码加入（Hello.join）
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
#[serde(default)]
pub struct Join {
    /// 这个空间开通、续费用过的任意一个兑换码
    pub code: String,
    /// 加入核对值：同步密码和 salt 算出来的（同步方案.md 第 5.1 节），服务器只存它的哈希
    pub proof: String,
}

/// 用手机加入（Hello.phone_ask / phone_poll）
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
#[serde(default)]
pub struct PhoneAsk {
    /// 这台电脑这次临时生成的 X25519 公钥（base64），手机把配对信息加密给它；两边核对的 4 个字也从它算
    pub key: String,
    /// 这台电脑叫什么（手机上显示「『工作室电脑』要加入」）
    pub name: String,
    /// phone_poll 时带上 phone_ask 拿到的 8 位数字
    pub code: String,
}

/// 用手机加入：服务器回的（Welcome.phone）
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
#[serde(default)]
pub struct PhoneJoin {
    /// 8 位数字，手机上输它
    pub code: String,
    /// 这个数字什么时候失效
    pub expires_at: String,
    /// 手机允许了：一次性的邀请码（和配对码里的一样，24 小时内有效）
    pub invite: Option<String>,
    /// 手机允许了：手机用 key 加密的空间密钥（nacl.box，和手机收件箱一样的格式：临时公钥 + nonce + 密文，base64），
    /// 服务器解不开
    pub sealed: Option<String>,
}

/// 握手第二条消息里服务器回的内容
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(default)]
pub struct Welcome {
    pub version: u32,
    pub space: Space,
    /// 服务器根证书的 SHA-256 指纹，电脑端添加手机时显示出来给 iPhone 核对
    pub ca_fingerprint: String,
    /// 手机网页另外绑了域名（宝塔、Nginx 反向代理过来，证书在那边）：`https://sync.example.com`，没绑是空的。
    /// 电脑添加手机时用它生成网址，手机不用再装根证书（需求-0.4 第 15.2 节）
    pub web_url: String,
    /// 服务器这份数据的「代」：第一次启动时随机生成，拿旧备份恢复（`restore-backup`）时换一个新的。
    /// 电脑记着上次见到的，变了就说明服务器回到了以前的样子：从头拉、把本机的全部（含附件）重新传一遍。
    /// 旧服务器没有这一项（空的），不管
    pub epoch: String,
    /// 协议 5 起：这个空间的账（套餐、到期、容量、设备数），每次连上都带；旧服务器没有
    pub account: Option<Account>,
    /// 协议 5 起：服务器不收这台电脑，说清楚为什么（兑换码不对、设备满了、到期了……），发完就断开。
    /// 只发给协议 5 以上的电脑，旧电脑照旧什么也收不到、直接断开
    pub refused: Option<Refusal>,
    /// 协议 6 起：回 Hello.ask_beta 的：公测现在怎样
    pub beta: Option<BetaInfo>,
    /// 协议 7 起：回 Hello.join_ask 的：这个空间的 salt
    pub join_salt: Option<String>,
    /// 协议 7 起：回 Hello.phone_ask / phone_poll 的
    pub phone: Option<PhoneJoin>,
}

/// 公测免费体验现在怎样（协议 6 起，Welcome.beta）：电脑开通前拿它决定显不显示「公测免费体验」
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
#[serde(default)]
pub struct BetaInfo {
    /// 能领：公测开着、没过截止日、还有名额
    pub open: bool,
    /// 还剩几个名额
    pub left: u64,
    /// 所有公测空间都用到这一天（这一天还能用），`YYYY-MM-DD`
    pub until: Option<String>,
    /// 套餐名，比如「公测体验」
    pub plan: String,
    /// 容量，None 不限
    pub quota: Option<u64>,
    /// 最多几台设备，None 不限
    pub max_devices: Option<u32>,
    /// 单个图片、单个附件最大多大（字节），None 不限
    pub max_image: Option<u64>,
    pub max_file: Option<u64>,
}

/// 服务器拒绝的原因：`code` 是下面 [`refuse`] 里的一个，电脑按它区分；`msg` 是给人看的一句话
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
#[serde(default)]
pub struct Refusal {
    pub code: String,
    pub msg: String,
}

/// 拒绝原因的 `code`（Refusal、Msg::Refused 里用）
pub mod refuse {
    /// 这台服务器没开兑换码（自建服务器）
    pub const REDEEM_OFF: &str = "redeem_off";
    /// 兑换码不存在（多半是输错了）
    pub const REDEEM_BAD: &str = "redeem_bad";
    /// 兑换码已经用过了
    pub const REDEEM_USED: &str = "redeem_used";
    /// 兑换码作废了（退款、泄露）
    pub const REDEEM_REVOKED: &str = "redeem_revoked";
    /// 过了兑换截止日
    pub const REDEEM_LATE: &str = "redeem_late";
    /// 这个 IP 一小时内输错太多次，停一小时
    pub const REDEEM_LOCKED: &str = "redeem_locked";
    /// 空间不会到期、用不着续这个（比如免费套餐不限容量）
    pub const REDEEM_NEEDLESS: &str = "redeem_needless";
    /// 空间里的电脑加手机到上限了（叮咪同步默认 10 台）
    pub const DEVICE_LIMIT: &str = "device_limit";
    /// 叮咪同步已到期：停止同步，数据在服务器上留 30 天
    pub const EXPIRED: &str = "expired";
    /// 到期太久，服务器上的数据删掉了：要用兑换码重新开通
    pub const DELETED: &str = "deleted";
    /// 空间满了：附件（图片、文件）传不上去，文字照常
    pub const FULL: &str = "full";
    /// 空间停用了（管理员冻结）
    pub const SPACE_OFF: &str = "space_off";
    /// 协议 6 起：公测没开、关了、过了截止日（或者服务器没开兑换码）
    pub const BETA_OFF: &str = "beta_off";
    /// 协议 6 起：公测名额领完了
    pub const BETA_FULL: &str = "beta_full";
    /// 协议 6 起：同一个网络（IP）今天领的公测太多了
    pub const BETA_IP: &str = "beta_ip";
    /// 协议 6 起：开通叮咪同步要先同意协议（Hello.terms 没带）
    pub const TERMS: &str = "terms";
    /// 协议 6 起：单个附件超过了套餐的大小（Account.max_image / max_file），服务器不收
    pub const TOO_BIG: &str = "too_big";
    /// 协议 6 起：邮箱格式不对
    pub const EMAIL_BAD: &str = "email_bad";
    /// 协议 6 起：服务器暂停了续费（试运营），还没到期的空间填码续费不收；到期以后照样能填码接着用
    pub const RENEW_OFF: &str = "renew_off";
    /// 协议 6 起：续上以后离最终到期超过了服务器设的天数（Account.renew_until_max），晚点再来续
    pub const RENEW_LIMIT: &str = "renew_limit";
    /// 协议 7 起：开通时填的兑换码已经开过空间，而且那个空间能用兑换码加入：电脑接到「输同步密码加入」
    pub const REDEEM_JOINABLE: &str = "redeem_joinable";
    /// 协议 7 起：兑换码加入时，这个码还没用过（该去「第一次开通」）
    pub const JOIN_UNUSED: &str = "join_unused";
    /// 协议 7 起：这个空间没开兑换码加入（同步密码不到 10 位、关掉了、这项上线前开通的还没补）
    pub const JOIN_OFF: &str = "join_off";
    /// 协议 7 起：兑换码加入时同步密码不对
    pub const JOIN_BAD: &str = "join_bad";
    /// 协议 7 起：这个空间兑换码加入试错太多次，锁一小时
    pub const JOIN_LOCKED: &str = "join_locked";
    /// 协议 7 起：用手机加入的数字过期了、手机上点了「不是我」，或者这个数字不存在
    pub const PHONE_GONE: &str = "phone_gone";
}

/// 一个空间的账（协议 5 起）：Welcome 和 AccountInfo 里带。电脑拿它显示套餐、到期、快满了，显示什么由电脑自己定
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
#[serde(default)]
pub struct Account {
    /// 空间的短编号，比如 `S-7K2M9`：「设置 → 同步」里显示，联系客服时报这个
    pub space: String,
    /// 服务器开了兑换码，也就是叮咪同步：电脑才显示套餐、到期、续费
    pub official: bool,
    /// 套餐名，比如「标准」
    pub plan: String,
    /// 现在这个套餐到哪天（这一天还能用），`YYYY-MM-DD`；None 是不会到期
    pub expires_at: Option<String>,
    /// 换档时剩下的时长：现在这个到期后接着用的，按顺序
    pub next: Vec<NextPlan>,
    /// 全部（含 next）到哪天；None 是不会到期
    pub until: Option<String>,
    /// 离 until 还有几天：那一天是 0，过了是负数；不会到期是 None
    pub days_left: Option<i64>,
    /// 到期了：停止同步（只能续费、问状态、要客服核对码）
    pub expired: bool,
    /// 到期了的话，服务器上的数据留到哪天（这一天整天都在，第二天删）
    pub delete_on: Option<String>,
    /// 现在该提醒的：`""` 不用提醒 / `expire_7d` 7 天内到期 / `expire_1d` 明天（或今天）到期 /
    /// `expired` 已到期 / `delete_7d` 离删除不到 7 天（delete_on 前 6 天起），按服务器的保留天数算
    pub remind: String,
    /// 用了多少（记录 + 附件的密文，字节）
    pub used: u64,
    /// 容量上限，None 是不限
    pub quota: Option<u64>,
    /// `ok` / `near_full` 用了 90% 以上 / `full` 满了（新的附件传不上去，文字照常）
    pub storage: String,
    /// 电脑加手机（没作废的）
    pub devices: u32,
    /// 最多几台，None 是不限
    pub max_devices: Option<u32>,
    /// 协议 6 起：单个图片最大多大（字节），None 是不限。电脑上超过的先压缩再传，压不下来就只留在那台电脑上
    pub max_image: Option<u64>,
    /// 协议 6 起：单个附件（文件）最大多大（字节），None 是不限。超过的不传，只留在那台电脑上
    pub max_file: Option<u64>,
    /// 协议 6 起：这个空间留的邮箱（只发给这个空间自己的设备），没留是 None
    pub email: Option<String>,
    /// 协议 6 起：现在是公测免费体验（到截止日就到期；填付费的兑换码马上换成付费套餐）
    pub beta: bool,
    /// 协议 7 起：这个空间开着兑换码加入（服务器上有盒子 3 和加入核对值）
    pub join: bool,
    /// 协议 7 起：用户（或后台）关掉了兑换码加入：换同步密码时不再自动打开，要在设置里打开
    pub join_off: bool,
    /// 协议 6 起：服务器暂停了续费（试运营）：还没到期时填码续费会被拒；到期了照样能填码接着用
    pub renew_off: bool,
    /// 协议 6 起：续费最多续到哪天（`YYYY-MM-DD`，今天 + 服务器设的天数），续上以后超过它的码不收；None 不限
    pub renew_until_max: Option<String>,
}

/// 现在这个套餐到期后接着用的
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
#[serde(default)]
pub struct NextPlan {
    pub plan: String,
    /// 用到哪天；None 是不会到期
    pub until: Option<String>,
}

/// 兑换码加入要的两样（SetupSpace、SetLock、JoinSet 里带）
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
#[serde(default)]
pub struct JoinLock {
    /// 盒子 3：用同步密码锁着的空间密钥，base64
    pub box3: String,
    /// 加入核对值（服务器只存哈希）
    pub proof: String,
}

/// 一个"空间"就是一个人的全部数据。下面几样由第一台设备生成，服务器只是保存，自己打不开：
/// 新设备用同步密码、salt 和空间密钥打开盒子 1，或者用恢复码打开盒子 2，拿到主密钥。
/// 都是 None 说明空间还没设同步密码（第一台设备还没来）
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
#[serde(default)]
pub struct Space {
    /// Argon2id 的参数和 salt：`argon2id$v=19$m=65536,t=3,p=1$<salt>`
    pub salt: Option<String>,
    /// 用同步密码（和空间密钥）锁着的主密钥，base64
    pub box1: Option<String>,
    /// 用恢复码锁着的主密钥和空间密钥，base64
    pub box2: Option<String>,
    /// 协议 7 起：用同步密码锁着的空间密钥（兑换码加入用），base64；没开兑换码加入是 None。
    /// 只在兑换码加入登记的那次连接里带，平时不带
    pub box3: Option<String>,
}

/// 空间里的一台电脑
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Pc {
    /// 设备公钥（base64url），作废时用
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub last_seen: Option<String>,
    /// 就是发问的这台
    pub me: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Phone {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub last_seen: Option<String>,
}

/// 手机发来、还没被电脑取走的一条
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct InboxItem {
    pub id: String,
    /// 哪部手机发的
    pub phone: String,
    pub size: u64,
    pub created_at: String,
}

/// 同步的一条记录：服务器只看得懂编号、版本和序号，内容是密文
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Record {
    pub uid: String,
    pub ver: String,
    /// 服务器给的序号，上传时不用填
    #[serde(default)]
    pub seq: u64,
    /// 加密后的内容（base64）
    pub blob: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Ack {
    pub uid: String,
    /// false 表示服务器上已经有更新的版本，这条没存
    pub ok: bool,
}

/// 握手之后的消息。电脑端发一条，服务器回一条。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "t", rename_all = "snake_case")]
// AccountInfo 带着整个账，比别的消息大一些；消息发完就扔，不值得装箱
#[allow(clippy::large_enum_variant)]
pub enum Msg {
    Ping,
    Pong,
    /// 第一台设备设好同步密码后，上传 salt、两个盒子和恢复凭证（服务器只存哈希）；空间已经设过时服务器回 Error。
    /// 协议 7 起：同步密码够长时一起带上盒子 3 和加入核对值（服务器只存哈希），开着兑换码加入
    SetupSpace {
        salt: String,
        box1: String,
        box2: String,
        recovery: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        join: Option<JoinLock>,
    },
    /// 换同步密码：新的 salt 和盒子 1，回 Ok。协议 7 起：盒子 3 和核对值是按 salt 算的，一起换；
    /// 没带（新密码不够长、关着兑换码加入、旧电脑）服务器就删掉原来的
    SetLock {
        salt: String,
        box1: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        join: Option<JoinLock>,
    },
    /// 协议 7 起：打开兑换码加入（这项上线前开通的空间补上、关了再打开），回 Ok；空间还没设同步密码回 Error。
    /// `salt` 是算盒子 3 时用的（1.0.8 起带）：服务器上的 salt 已经换了（别的电脑同时换了同步密码）就不收，回 Error
    JoinSet {
        join: JoinLock,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        salt: Option<String>,
    },
    /// 协议 7 起：关掉兑换码加入：服务器删掉盒子 3 和核对值，回 Ok
    JoinOff,
    /// 协议 7 起：删掉服务器上的这个空间（用户自己删，需求-1.x 第 35.3 节），回 Ok 后服务器断开所有设备。只有叮咪同步有。
    /// `proof` 是加入核对值（1.0.8 起带）：空间开着兑换码加入时，服务器拿它再核对一次同步密码
    SpaceDelete {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        proof: Option<String>,
    },
    /// 换恢复码：新的盒子 2 和恢复凭证，旧的作废，回 Ok
    SetRecovery { box2: String, recovery: String },
    Ok,
    Error { msg: String },
    /// 协议 5 起：服务器不做这件事、原因说得清（code 见 [`refuse`]）。只回给协议 5 以上的电脑，旧的照旧回 Error
    Refused { code: String, msg: String },

    /// 协议 5 起：续费（填兑换码），回 AccountInfo；同一档接在到期日后面，换档规则见需求-1.0 第 18.3 节。
    /// 协议 6 起带上同意的协议版本（terms），服务器更新记录
    Renew {
        code: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        terms: Option<String>,
    },
    /// 协议 5 起：问这个空间的账，回 AccountInfo
    AccountGet,
    AccountInfo { account: Account },
    /// 协议 5 起：要一个客服核对码（6 位数，10 分钟有效），回 Support
    SupportNew,
    Support { code: String, expires_at: String },
    /// 协议 6 起：设置（或改）这个空间的邮箱，回 AccountInfo；格式不对回 Refused email_bad。只有叮咪同步有
    EmailSet { email: String },
    /// 协议 6 起：删掉邮箱（以后不再发通知），回 AccountInfo
    EmailDel,

    /// 给一部新手机发令牌，回 PhoneAdded
    PhoneAdd { name: String },
    PhoneAdded { id: String, token: String },
    /// 回 Phones
    PhoneList,
    Phones { phones: Vec<Phone> },
    PhoneRevoke { id: String },
    /// 空间里的电脑（设备满了、电脑丢了时在「设置 → 同步」里作废不用的，需求-1.0 第 30.4 节），回 Pcs
    PcList,
    Pcs { pcs: Vec<Pc> },
    /// 作废空间里的另一台电脑（id 是它的设备公钥），回 Ok；不能作废自己（自己用「停止同步」）
    PcRevoke { id: String },

    /// 给另一台电脑生成配对码（同一个空间，24 小时内有效，用一次作废），回 Invited
    InviteNew,
    Invited { token: String, expires_at: String },

    /// 等新东西：有就马上回（版本 1 回 InboxNotify，版本 2 回 Changed），等 25 秒还没有就回 Pong（顺便当心跳）
    Wait,
    InboxNotify,
    /// 手机发来了东西，或者别的电脑改了数据
    Changed,

    /// 上传改过的记录，回 Acks。服务器上的版本更新时不覆盖
    Push { items: Vec<Record> },
    Acks { acks: Vec<Ack> },
    /// 拉取序号大于 since 的记录，回 Records；more 表示还没拉完
    Pull { since: u64 },
    Records { items: Vec<Record>, more: bool },
    /// 图片：按块上传（done 表示传完了）、下载、删除。uid 是图片的编号，内容是密文
    FilePut { uid: String, offset: u64, data: String, done: bool },
    FileGet { uid: String, offset: u64 },
    FileChunk { uid: String, offset: u64, data: String, done: bool },
    FileDel { uid: String },
    /// 回 InboxItems
    InboxList,
    InboxItems { items: Vec<InboxItem> },
    /// 从 offset 开始下载一块，回 InboxChunk；data 是 base64，done 表示这是最后一块
    InboxGet { id: String, offset: u64 },
    InboxChunk { id: String, offset: u64, data: String, done: bool },
    /// 电脑已经存好了，服务器上删掉
    InboxDel { id: String },
}

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Noise(snow::Error),
    Json(serde_json::Error),
    Protocol(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "网络错误：{e}"),
            Error::Noise(e) => write!(f, "加密连接出错：{e}"),
            Error::Json(e) => write!(f, "消息格式不对：{e}"),
            Error::Protocol(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<snow::Error> for Error {
    fn from(e: snow::Error) -> Self {
        Error::Noise(e)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Json(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    /// 1.0.8 给 SpaceDelete、JoinSet 加的字段：旧电脑不带也认；新电脑带了，旧服务器（unit 变体）也认得
    #[test]
    fn new_fields_stay_compatible() {
        let m: Msg = serde_json::from_str(r#"{"t":"space_delete"}"#).unwrap();
        assert_eq!(m, Msg::SpaceDelete { proof: None });
        let m: Msg = serde_json::from_str(r#"{"t":"join_set","join":{"box3":"b","proof":"p"}}"#).unwrap();
        assert_eq!(m, Msg::JoinSet { join: JoinLock { box3: "b".into(), proof: "p".into() }, salt: None });
        let s = serde_json::to_string(&Msg::SpaceDelete { proof: None }).unwrap();
        assert_eq!(s, r#"{"t":"space_delete"}"#, "不带时和以前发的一模一样");

        #[derive(Deserialize, Debug, PartialEq)]
        #[serde(tag = "t", rename_all = "snake_case")]
        enum Old {
            SpaceDelete,
        }
        let s = serde_json::to_string(&Msg::SpaceDelete { proof: Some("p".into()) }).unwrap();
        assert_eq!(serde_json::from_str::<Old>(&s).unwrap(), Old::SpaceDelete);
    }
}