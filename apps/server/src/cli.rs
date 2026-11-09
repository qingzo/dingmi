//! 命令行：dingmi-server（main.rs）直接用。在它上面加东西的程序（比如叮咪同步的官方版）也用这一份，
//! 通过 `Extra` 加上自己的命令、扩展点（ext.rs）和 run 起来后要跑的东西。
//! 管理用的命令（spaces、quota、disable……）调的是 admin.rs。

use crate::admin::{self, fmt_size, SpaceInfo};
use crate::ext::{Ext, SelfHosted};
use crate::net::{self, Server};
use crate::store::{self, Store, INVITE_HOURS};
use crate::{init, tls};
use std::error::Error;
use std::future::Future;
use std::net::IpAddr;
use std::pin::Pin;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, OnceLock};

const DEFAULT_DATA: &str = "/var/lib/dingmi";
const DEFAULT_PORT: u16 = 7788;

const USAGE: &str = "dingmi-server：叮咪的同步服务器

用法：dingmi-server <命令> [选项]

命令：
  init --ip <公网IP> [--port 7788]   第一次使用：生成服务器密钥、证书和数据库。
                                     以后换了 IP 或端口，再执行一次就行，密钥和数据不变
  run                                启动服务（在宝塔进程守护或 systemd 里用这个）
  invite [--new-space | --space <空间>]
                                     生成一个配对码给电脑用，24 小时内有效，只能用一次。
                                     不写就是最近建的那个空间；--new-space 新建一个空间
  spaces                             列出空间：备注名、几台电脑几部手机、占了多少、最后活动
  devices [空间]                     列出电脑和手机（带编号）
  revoke <名字或编号前几位>            作废一台电脑或一部手机，它就再也连不上了；同名的有好几台时写编号
  note <空间> <备注名>               给空间起个备注名，比如「爸妈」（只在服务器上看）；--off 去掉
  quota <空间> <大小>                容量上限，比如 2G、500M；off 不限。满了新的图片和文件传不上去（文字照常），电脑上提示「空间满了」
  disable <空间> --yes               停用空间：电脑和手机都连不上，30 天（config 能改）后连数据一起删掉，之前可以 restore
  restore <空间>                     恢复停用的空间
  backups                            列出每天的备份（数据目录的 backups 里，默认留 14 天）
  config [delete-days <天数>] [backup-days <天数>] [default-quota <大小>]
                                     看或改：停用的空间几天后删掉（默认 30）、备份留几天（默认 14）、
                                     新空间的默认容量上限（比如 1G；off 不限，只管以后新建的空间）
  restore-backup <日期>              拿某一天的备份恢复数据库（先停掉服务）：现在的数据库改名留着，
                                     电脑和手机连上时自己从头同步、把本机的重新传一遍（附件也是）
  ca                                 显示根证书指纹和 iPhone 下载地址
  ca --new --yes                     换一张只认这台服务器 IP 的根证书（带名称约束，私钥泄露也冒充不了别的网站）。
                                     换了以后用 IP 打开手机网页的手机要重装证书；绑了域名的手机不受影响。先停掉服务
  web <https://域名>                 手机网页另外绑了域名（宝塔、Nginx 反向代理到 https://127.0.0.1:7788）时写上，
                                     电脑添加手机时就用这个网址，手机不用装根证书；web --off 取消，只写 web 看现在的
  version                            显示版本（开源版还是官方版）
  edition --self-hosted --yes        叮咪同步官方版用过的数据库开源版不打开；确定要拿开源版接着用时执行这个去掉标记

<空间> 写备注名，或者编号前几位（至少 4 位，spaces 里能看到）。

选项：
  --data <目录>   数据目录，默认 /var/lib/dingmi
";

pub type Res = Result<(), Box<dyn Error>>;

/// 在这个命令行上加东西的程序（比如官方版）多出来的。都是编译时定的函数，配置改不了
pub struct Extra {
    /// 接在用法说明后面
    pub usage: &'static str,
    /// 多出来的命令：认识就处理、返回 Some（在自带的命令之前问，所以也能接管自带的命令）；`args` 是整个命令行（不含程序名）
    pub command: fn(cmd: &str, args: &[String], dir: &Path) -> Option<Res>,
    /// 扩展点（ext.rs）：run 起来的服务器用它；命令行打开数据库时也先让它建表（Ext::open）
    pub ext: fn() -> Arc<dyn Ext>,
    /// run 起来以后另外要跑的（已经在 tokio 运行时里，可以直接 spawn）
    pub on_run: fn(Arc<Server>),
    /// 一条命令做成了以后（`args` 是整个命令行，不含程序名）
    pub on_done: fn(dir: &Path, args: &[String]),
    /// 命令行里列出空间（spaces、support……）时接在那一行后面的（比如套餐和到期日），空的就不加
    pub space_note: fn(store: &Store, space: &str) -> String,
    /// 命令行里写空间时，除了备注名、编号开头还认什么（比如短编号）：认出来返回空间编号
    pub find_space: fn(store: &Store, who: &str) -> Option<String>,
    /// `config` 不带参数时接在后面显示的
    pub config_show: fn(store: &Store),
    /// run 怎么收连接。None 是开源版的样子：直连，自签证书，在 init 的端口上等电脑和手机；
    /// Some 是在宝塔后面（官方版）：服务器用 `Server::load_proxied` 起（不要 IP、证书），听哪、怎么分请求都交给它，
    /// 它返回了程序就退出（返回 Err 是起不来，比如端口打不开）。`on_run` 照样在它之前调
    pub serve: Option<Serve>,
}

/// 见 Extra.serve
pub type Serve = fn(Arc<Server>) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>>;

static EXTRA: OnceLock<&'static Extra> = OnceLock::new();

fn extra() -> Option<&'static Extra> {
    EXTRA.get().copied()
}

/// 用哪个扩展点：有 Extra 用它的，没有用开源版的 SelfHosted
fn ext() -> Arc<dyn Ext> {
    match extra() {
        Some(x) => (x.ext)(),
        None => Arc::new(SelfHosted),
    }
}

/// 开源版的命令说明。在宝塔后面跑的（官方版，Extra.serve）没有 `init --ip`、`ca`、`web`，这几条连同下面的续行一起去掉，
/// 它自己的说法在 Extra.usage 里
fn base_usage(proxied: bool) -> String {
    if !proxied {
        return USAGE.to_string();
    }
    let mut out = String::new();
    let mut skip = false;
    for line in USAGE.lines() {
        if line.starts_with("  ") && !line.starts_with("   ") {
            skip = ["  init --ip", "  ca ", "  ca\t", "  web "].iter().any(|p| line.starts_with(p)) || line.trim() == "ca";
        } else if !line.starts_with("   ") {
            skip = false;
        }
        if !skip {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

pub fn main(extra: Option<&'static Extra>) -> ExitCode {
    if let Some(x) = extra {
        let _ = EXTRA.set(x);
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let opt = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let dir = PathBuf::from(opt("--data").unwrap_or_else(|| DEFAULT_DATA.into()));
    let usage = || print!("{}{}", base_usage(extra.is_some_and(|x| x.serve.is_some())), extra.map(|x| x.usage).unwrap_or(""));
    if let (Some(x), Some(cmd)) = (extra, args.first()) {
        if let Some(r) = (x.command)(cmd, &args, &dir) {
            // 它自己的命令做成了也告诉它（记进后台的操作日志）
            if r.is_ok() {
                (x.on_done)(&dir, &args);
            }
            return finish(r);
        }
    }
    let has = |name: &str| args.iter().any(|a| a == name);
    // 命令后面第 n 个不是选项的词（跳过 --data 这些带值的选项）
    let word = |n: usize| -> Option<String> {
        let mut words = vec![];
        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "--data" | "--ip" | "--port" | "--space" => i += 2,
                a if a.starts_with("--") => i += 1,
                a => {
                    words.push(a.to_string());
                    i += 1;
                }
            }
        }
        words.get(n).cloned()
    };
    let need = |n: usize, what: &str| word(n).ok_or_else(|| -> Box<dyn Error> { what.into() });
    let r = match args.first().map(String::as_str) {
        Some("init") => init_cmd(&dir, opt("--ip"), opt("--port")),
        Some("run") => run(&dir, extra),
        Some("invite") => invite(&dir, has("--new-space"), opt("--space")),
        Some("spaces") => spaces(&dir),
        Some("devices") => devices(&dir, word(0)),
        Some("revoke") => {
            need(0, "要作废哪台？写上名字或编号前几位，可以先用 devices 看看").and_then(|w| revoke(&dir, &w))
        }
        Some("note") => need(0, "写上空间和备注名，比如：note 1a2b 爸妈")
            .and_then(|sp| note(&dir, &sp, if has("--off") { Some(String::new()) } else { word(1) })),
        Some("quota") => need(0, "写上空间和大小，比如：quota 爸妈 2G").and_then(|sp| quota(&dir, &sp, word(1))),
        Some("disable") => {
            need(0, "要停用哪个空间？写备注名或编号前几位").and_then(|sp| disable(&dir, &sp, has("--yes")))
        }
        Some("restore") => need(0, "要恢复哪个空间？写备注名或编号前几位").and_then(|sp| restore(&dir, &sp)),
        Some("backups") => backups(&dir),
        Some("config") => config(&dir, word(0), word(1)),
        Some("restore-backup") => need(0, "写上日期，比如：restore-backup 2026-11-05（用 backups 看看有哪些）")
            .and_then(|day| restore_backup(&dir, &day)),
        Some("ca") if has("--new") => ca_new(&dir, has("--yes")),
        Some("ca") => ca(&dir),
        Some("web") => web(&dir, if has("--off") { Some("--off".to_string()) } else { word(0) }.as_deref()),
        Some("version" | "--version" | "-V") => {
            println!("dingmi-server {}（{}）", env!("CARGO_PKG_VERSION"), ext().edition());
            Ok(())
        }
        Some("edition") => edition(&dir, has("--self-hosted"), has("--yes")),
        Some("help" | "--help" | "-h") => {
            usage();
            Ok(())
        }
        _ => {
            usage();
            return ExitCode::from(2);
        }
    };
    // 加了东西的程序（官方版）：做成了的命令也告诉它（记进后台的操作日志）
    if let (Ok(()), Some(x)) = (&r, extra) {
        (x.on_done)(&dir, &args);
    }
    finish(r)
}

fn finish(r: Res) -> ExitCode {
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("出错了：{e}");
            ExitCode::FAILURE
        }
    }
}

fn init_cmd(dir: &Path, ip: Option<String>, port: Option<String>) -> Res {
    let ip: IpAddr = ip
        .ok_or("要写上服务器的公网 IP，比如：dingmi-server init --ip 1.2.3.4")?
        .parse()
        .map_err(|_| "IP 格式不对")?;
    let port: u16 = match port {
        Some(p) => p.parse().map_err(|_| "端口格式不对")?,
        None => Store::open(dir)
            .ok()
            .and_then(|s| s.kv_get("port"))
            .and_then(|p| p.parse().ok())
            .unwrap_or(DEFAULT_PORT),
    };
    init(dir, ip, port)?;
    opened(dir)?;
    println!("初始化好了。");
    println!("  数据目录：{}", dir.display());
    println!("  地址：{ip}:{port}");
    println!("  根证书指纹（SHA-256）：{}", tls::fingerprint(&tls::ca_der(dir)?));
    if tls::ca_limit(dir).is_some_and(|ips| !ips.contains(&ip)) {
        println!();
        println!("注意：现在的根证书只认以前的 IP，用新 IP 打开手机网页会有证书警告。");
        println!("停掉服务后执行 dingmi-server ca --new --yes 换一张认新 IP 的，手机重装一次证书（绑了域名的手机不用管）。");
    }
    println!();
    println!("下一步：启动服务（dingmi-server run，或者在宝塔进程守护管理器里添加），");
    println!("然后用 dingmi-server invite 生成配对码。");
    Ok(())
}

fn run(dir: &Path, extra: Option<&Extra>) -> Res {
    // 工作线程按 CPU 个数（至少 2、最多 8）：数据库和小块的文件读写是直接在这些线程上做的，
    // 只有两个的话，两个慢的请求就能让别的连接都等着（整个附件的读写另外放到 spawn_blocking 上）
    let threads = std::thread::available_parallelism().map_or(2, |n| n.get()).clamp(2, 8);
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(threads).enable_all().build()?;
    rt.block_on(async {
        // 官方版（Extra.serve）在宝塔后面：不要 IP、证书，自己收连接
        if let Some((x, serve)) = extra.and_then(|x| Some((x, x.serve?))) {
            let server = Server::load_proxied(dir, ext())?;
            if let Some(n) = raise_open_files() {
                net::log(format!("最多同时开 {n} 个连接（每台连着的电脑占一两个）"));
            }
            (x.on_run)(server.clone());
            tokio::spawn(server.clone().daily());
            return serve(server).await.map_err(Into::into);
        }
        let server = Server::load_with(dir, ext())?;
        let port: u16 = server.store.kv_get("port").and_then(|p| p.parse().ok()).unwrap_or(DEFAULT_PORT);
        // 在开发电脑上测试时设 DINGMI_SERVER_BIND=127.0.0.1，只听本机，Windows 防火墙不会弹窗
        let bind = std::env::var("DINGMI_SERVER_BIND").unwrap_or_else(|_| "0.0.0.0".into());
        let listener = tokio::net::TcpListener::bind((bind.as_str(), port))
            .await
            .map_err(|e| format!("端口 {port} 打不开：{e}"))?;
        net::log(format!("dingmi-server {} 启动了，在 {port} 端口等待连接", env!("CARGO_PKG_VERSION")));
        if let Some(n) = raise_open_files() {
            net::log(format!("最多同时开 {n} 个连接（每台连着的电脑占一两个）"));
        }
        if let Some(x) = extra {
            (x.on_run)(server.clone());
        }
        tokio::spawn(server.clone().daily());
        server.serve(listener).await;
        Ok(())
    })
}

/// Linux 默认一个程序最多同时开 1024 个文件和连接，一台电脑连着就占一个，900 来台就满了。
/// 启动时调到系统允许的最大（软上限调到硬上限），不用另外去改系统配置。返回调完是多少
#[cfg(unix)]
fn raise_open_files() -> Option<u64> {
    let mut r = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
    // SAFETY: getrlimit / setrlimit 只读写这个结构体
    unsafe {
        if libc::getrlimit(libc::RLIMIT_NOFILE, &mut r) != 0 {
            return None;
        }
        if r.rlim_cur < r.rlim_max {
            let want = libc::rlimit { rlim_cur: r.rlim_max, rlim_max: r.rlim_max };
            if libc::setrlimit(libc::RLIMIT_NOFILE, &want) == 0 {
                r = want;
            }
        }
    }
    Some(r.rlim_cur as u64)
}

#[cfg(not(unix))]
fn raise_open_files() -> Option<u64> {
    None
}

/// 打开数据目录里的数据库（要先 init）；加了扩展点的程序先让扩展点建它的表
pub fn opened(dir: &Path) -> Result<Store, Box<dyn Error>> {
    if !dir.join(store::DB_FILE).exists() {
        return Err(format!("{} 里还没有数据，先执行 dingmi-server init --ip <公网IP>", dir.display()).into());
    }
    let store = Store::open(dir)?;
    ext().open(&store)?;
    Ok(store)
}

/// `edition`：看这份数据带没带官方版的标记；`--self-hosted --yes` 去掉它（以后开源版能打开）
fn edition(dir: &Path, self_hosted: bool, yes: bool) -> Res {
    if !dir.join(store::DB_FILE).exists() {
        return Err(format!("{} 里还没有数据", dir.display()).into());
    }
    // 不经过 opened：带着标记时开源版的 opened 打不开
    let store = Store::open(dir)?;
    let official = store.kv_get(crate::ext::EDITION_KEY).as_deref() == Some(crate::ext::OFFICIAL);
    if !self_hosted {
        println!("这个程序：{}", ext().edition());
        if official {
            println!("这份数据：叮咪同步官方版用过（开源版不打开）。确定要拿开源版接着用：edition --self-hosted --yes");
        } else {
            println!("这份数据：没有官方版的标记，开源版能打开");
        }
        return Ok(());
    }
    if !official {
        println!("这份数据本来就没有官方版的标记，开源版能打开。");
        return Ok(());
    }
    if !yes {
        return Err("去掉以后开源版就能打开这份数据，叮咪同步的兑换码、到期、设备上限、连接日志都不管了（到期的空间会接着同步）。\
确定的话加 --yes"
            .into());
    }
    crate::ext::clear_edition(&store)?;
    println!("去掉了：开源版能打开这份数据了。官方版下次启动时会再写上。");
    Ok(())
}

/// 命令行里写的空间：备注名、编号开头（至少 4 位），加了东西的程序还能认别的（Extra.find_space）
fn find(store: &Store, who: &str) -> Result<SpaceInfo, String> {
    match extra().and_then(|x| (x.find_space)(store, who.trim())) {
        Some(id) => admin::find_space(store, &id),
        None => admin::find_space(store, who),
    }
}

fn invite(dir: &Path, new_space: bool, space: Option<String>) -> Res {
    let store = opened(dir)?;
    let space = match (new_space, space) {
        (true, _) => None,
        (false, Some(who)) => Some(find(&store, &who)?.id),
        // 没说哪个空间：最近建的那个（个人用通常只有一个），一个都没有就新建
        (false, None) => store.latest_space(),
    };
    let inv = admin::invite(&store, space.as_deref())?;
    print_invite(&inv);
    Ok(())
}

/// 显示生成好的配对码和怎么用
pub fn print_invite(inv: &admin::Invite) {
    println!("配对码（{INVITE_HOURS} 小时内有效，只能用一次）：");
    println!();
    println!("{}", inv.code);
    println!();
    println!("在叮咪的「设置 → 同步」里粘贴这一整行。");
    if inv.set_up {
        println!("这个空间已经在用了，服务器上生成的配对码不带空间密钥：新电脑粘贴后要输恢复码。");
        println!("手边有已经同步的电脑的话，在那台电脑的「设置 → 同步」里生成配对码更方便，只要输同步密码。");
    }
}

fn short(id: &str) -> &str {
    &id[..8.min(id.len())]
}

/// 列出空间时的一行。`more` 接在编号后面（加了东西的程序给的，比如短编号、套餐和到期日），空的就不加
pub fn space_line(s: &SpaceInfo, more: &str) -> String {
    let used = match s.quota {
        Some(q) => format!("{} / {}{}", fmt_size(s.used()), fmt_size(q), if s.full() { "（满了）" } else { "" }),
        None => fmt_size(s.used()),
    };
    let state = match &s.delete_on {
        Some(day) => format!("  （停用中，{day} 删掉）"),
        None => String::new(),
    };
    format!(
        "{}  编号 {}{more}  电脑 {}  手机 {}  占用 {used}  最后活动 {}  建立于 {}{state}",
        s.note.as_deref().unwrap_or("（没起备注名）"),
        short(&s.id),
        s.computers,
        s.phones,
        s.last_active.as_deref().unwrap_or("从未"),
        s.created_at
    )
}

/// 加了东西的程序接在空间那一行后面的（Extra.space_note）
fn more(store: &Store, space: &str) -> String {
    extra().map(|x| (x.space_note)(store, space)).unwrap_or_default()
}

fn spaces(dir: &Path) -> Res {
    let store = opened(dir)?;
    let list = admin::spaces(&store);
    if list.is_empty() {
        println!("还没有空间。用 dingmi-server invite 生成配对码，第一台电脑配对时就有了。");
    }
    for s in &list {
        println!("{}", space_line(s, &more(&store, &s.id)));
    }
    Ok(())
}

fn devices(dir: &Path, space: Option<String>) -> Res {
    let store = opened(dir)?;
    let list = match space {
        Some(who) => vec![find(&store, &who)?],
        None => admin::spaces(&store),
    };
    if list.is_empty() {
        println!("还没有登记过电脑。用 dingmi-server invite 生成配对码。");
    }
    let gone = |revoked: bool| if revoked { "  （已作废）" } else { "" };
    for s in list {
        match &s.note {
            Some(n) => println!("空间 {n}（{}）", short(&s.id)),
            None => println!("空间 {}", short(&s.id)),
        }
        let (pcs, mobiles) = (admin::computers(&store, &s.id), admin::phones(&store, &s.id));
        if pcs.is_empty() && mobiles.is_empty() {
            println!("  还没有电脑和手机");
        }
        for d in pcs {
            println!(
                "  电脑 {}  编号 {}  登记于 {}  最后连接 {}{}",
                d.name,
                short(&d.pubkey),
                d.created_at,
                d.last_seen.as_deref().unwrap_or("从未"),
                gone(d.revoked)
            );
        }
        for p in mobiles {
            println!(
                "  手机 {}  编号 {}  添加于 {}  最后连上 {}{}",
                p.name,
                short(&p.id),
                p.created_at,
                p.last_seen.as_deref().unwrap_or("从未"),
                gone(p.revoked)
            );
        }
    }
    Ok(())
}

fn revoke(dir: &Path, who: &str) -> Res {
    let store = opened(dir)?;
    let what = admin::revoke(&store, who)?;
    println!("已作废：{what}。正在连着的下一次同步时就断开了。");
    Ok(())
}

fn note(dir: &Path, who: &str, note: Option<String>) -> Res {
    let store = opened(dir)?;
    let note = note.ok_or("写上备注名，比如：note 1a2b 爸妈；去掉备注名用 --off")?;
    let s = find(&store, who)?;
    admin::set_note(&store, &s.id, &note)?;
    if note.trim().is_empty() {
        println!("去掉了空间 {} 的备注名。", short(&s.id));
    } else {
        println!("空间 {} 的备注名改成了「{}」。", short(&s.id), note.trim());
    }
    Ok(())
}

fn quota(dir: &Path, who: &str, size: Option<String>) -> Res {
    let store = opened(dir)?;
    let size = size.ok_or("写上大小，比如：quota 爸妈 2G；不限写 off")?;
    let s = find(&store, who)?;
    let q = admin::parse_size(&size)?;
    admin::set_quota(&store, &s.id, q)?;
    match q {
        None => println!("「{}」不限容量了。", s.label()),
        Some(q) => {
            println!("「{}」的容量上限改成了 {}，现在用了 {}。", s.label(), fmt_size(q), fmt_size(s.used()));
            if s.used() >= q {
                println!("已经超了：存着的不会删，只是新的图片和文件传不上去（文字照常同步），电脑上会提示「空间满了」。");
            }
        }
    }
    Ok(())
}

fn disable(dir: &Path, who: &str, yes: bool) -> Res {
    let store = opened(dir)?;
    let s = find(&store, who)?;
    if !yes {
        println!("{}", space_line(&s, &more(&store, &s.id)));
        println!();
        let days = admin::days(&store, &admin::DELETE_DAYS);
        println!("停用以后这个空间的电脑和手机都连不上，{days} 天后连数据一起删掉，之前可以用 restore 恢复。");
        println!("确定的话加上 --yes 再执行一次：dingmi-server disable {who} --yes");
        return Ok(());
    }
    let day = admin::disable(&store, &s.id)?;
    println!("停用了「{}」。{day} 删掉，之前可以 dingmi-server restore {who} 恢复。", s.label());
    Ok(())
}

fn restore(dir: &Path, who: &str) -> Res {
    let store = opened(dir)?;
    let s = find(&store, who)?;
    admin::restore(&store, &s.id)?;
    println!("恢复了「{}」，电脑和手机下次连上就照常同步。", s.label());
    Ok(())
}

fn config(dir: &Path, what: Option<String>, value: Option<String>) -> Res {
    let store = opened(dir)?;
    let item = match what.as_deref() {
        None => None,
        Some("delete-days") => Some(&admin::DELETE_DAYS),
        Some("backup-days") => Some(&admin::BACKUP_DAYS),
        Some("default-quota") => {
            let q = admin::parse_size(&value.ok_or("写上大小，比如：config default-quota 1G；不限写 off")?)?;
            admin::set_default_quota(&store, q)?;
            match q {
                Some(q) => println!("改好了：以后新建的空间默认最多 {}，已有的不变。", fmt_size(q)),
                None => println!("改好了：以后新建的空间默认不限容量。"),
            }
            return Ok(());
        }
        Some(w) => return Err(format!("不认识「{w}」，能改的是 delete-days、backup-days、default-quota（help 里有）").into()),
    };
    if let Some(d) = item {
        let n: i64 = value.and_then(|v| v.parse().ok()).ok_or("写上天数，比如：config delete-days 30")?;
        admin::set_days(&store, d, n)?;
        println!("改好了：{}，{n} 天。", d.name);
        if d.key == admin::DELETE_DAYS.key {
            println!("已经停用的空间也按新的天数算，服务器每天收尾时（每天一次）照这个删。");
        }
        return Ok(());
    }
    for d in [&admin::DELETE_DAYS, &admin::BACKUP_DAYS] {
        println!("{}：{} 天（默认 {}）", d.name, admin::days(&store, d), d.default);
    }
    println!("新空间的默认容量上限：{}", admin::default_quota(&store).map(fmt_size).unwrap_or_else(|| "不限".into()));
    if let Some(x) = extra() {
        (x.config_show)(&store);
    }
    Ok(())
}

fn restore_backup(dir: &Path, day: &str) -> Res {
    opened(dir)?;
    // 服务还在跑的话，它占着端口：先停掉再换数据库。官方版在宝塔后面，听的是本机的 listen_port（官方版记的）
    let (bind, port) = {
        // 看完就关掉数据库（下面要换掉它）
        let store = Store::open(dir).ok();
        let kv = |k: &str| store.as_ref().and_then(|s| s.kv_get(k)).and_then(|p| p.parse::<u16>().ok());
        match kv("listen_port") {
            Some(p) => ("127.0.0.1", p),
            None => ("0.0.0.0", kv("port").unwrap_or(DEFAULT_PORT)),
        }
    };
    if std::net::TcpListener::bind((bind, port)).is_err() {
        return Err(format!("端口 {port} 还被占着，多半是服务还在跑：先在宝塔「Go 项目」里停止 dingmi，再执行这一条").into());
    }
    let kept = admin::restore_backup(dir, day)?;
    println!("恢复好了：数据库换成了 {day} 的备份，原来的改名留着（{kept}）。");
    println!("现在启动服务。电脑和手机连上时会自己从头同步，把本机的东西（含附件）重新传一遍，备份之后新加的也就回来了。");
    Ok(())
}

fn backups(dir: &Path) -> Res {
    opened(dir)?;
    let list = admin::backups(dir);
    if list.is_empty() {
        println!("还没有备份。服务启动后每天备份一次数据库，放在 {}。", dir.join("backups").display());
    }
    for (day, size) in list {
        println!(
            "{day}  {}  {}",
            fmt_size(size),
            dir.join("backups").join(format!("dingmi-server-{day}.db")).display()
        );
    }
    Ok(())
}

/// 换一张只认这台服务器 IP 的根证书（带名称约束）。服务在跑的话它还拿着旧证书，所以要先停掉
fn ca_new(dir: &Path, yes: bool) -> Res {
    let store = opened(dir)?;
    let ip: IpAddr = store.kv_get("ip").and_then(|s| s.parse().ok()).ok_or("还没有初始化，先执行 dingmi-server init --ip <公网IP>")?;
    if !yes {
        println!("会生成一张新的根证书，只能给 {ip} 签证书：根证书私钥万一泄露，别人也冒充不了别的网站。");
        println!("换了以后，用 https://{ip}:端口 打开手机网页的手机都要重装一次证书（旧的会有证书警告）；");
        println!("绑了域名（dingmi-server web）的手机用的是域名的证书，不受影响。旧的根证书文件改名留着。");
        println!("服务器以后换了 IP，要再执行一次这条、手机再重装。");
        println!("确定的话先停掉服务，加上 --yes 再执行一次：dingmi-server ca --new --yes");
        return Ok(());
    }
    let port: u16 = store.kv_get("port").and_then(|p| p.parse().ok()).unwrap_or(DEFAULT_PORT);
    if std::net::TcpListener::bind(("0.0.0.0", port)).is_err() {
        return Err(format!("端口 {port} 还被占着，多半是服务还在跑：先停掉服务，再执行这一条").into());
    }
    let tag = tls::renew_ca(dir, ip, &store)?;
    println!("换好了，只认 {ip}。旧的根证书改名留着（ca.crt.pem{tag}、ca.key.pem{tag}）。");
    println!("新的根证书指纹（SHA-256）：{}", tls::fingerprint(&tls::ca_der(dir)?));
    println!("现在启动服务，然后手机重装证书：iPhone 先在「设置 → 通用 → VPN 与设备管理」里删掉旧的描述文件，");
    println!("再用 Safari 打开 http://{ip}:{port}/ca 安装、到「证书信任设置」里打开；安卓下载 http://{ip}:{port}/ca.crt 重装。");
    Ok(())
}

/// 手机网页绑的域名，存在 kv 的 web_url；电脑每次连上时带过去，改了不用重启服务
fn web(dir: &Path, arg: Option<&str>) -> Res {
    let store = opened(dir)?;
    match arg {
        None => match store.kv_get("web_url").filter(|u| !u.is_empty()) {
            Some(u) => println!("手机网页的网址：{u}"),
            None => println!("手机网页没有另外绑域名，用的是 https://IP:端口（要装根证书）"),
        },
        Some("--off") => {
            store.kv_set("web_url", "")?;
            println!("取消了。之后在电脑上添加的手机用 https://IP:端口 打开，要装根证书。");
        }
        Some(u) => {
            let url = admin::set_web_url(&store, u)?;
            println!("记下了：{url}");
            println!("之后在电脑上「设置 → 同步 → 手机」添加的手机用这个网址打开，不用装根证书。");
            println!("已经添加过的手机还是原来的网址，想换就在电脑上作废、重新添加。");
            println!("先确认浏览器打开 {url}/m/ 能看到叮咪的页面、没有证书警告。");
        }
    }
    Ok(())
}

fn ca(dir: &Path) -> Res {
    let store = opened(dir)?;
    let ip = store.kv_get("ip").unwrap_or_default();
    let port = store.kv_get("port").unwrap_or_else(|| DEFAULT_PORT.to_string());
    println!("根证书指纹（SHA-256）：{}", tls::fingerprint(&tls::ca_der(dir)?));
    match tls::ca_limit(dir) {
        Some(ips) => println!(
            "这张根证书只认：{}",
            ips.iter().map(|i| i.to_string()).collect::<Vec<_>>().join("、")
        ),
        None => println!("这张根证书没有名称约束（想换成只认这台服务器 IP 的，见 dingmi-server ca --new）"),
    }
    println!("iPhone 下载地址：http://{ip}:{port}/ca");
    println!("安卓下载地址：http://{ip}:{port}/ca.crt");
    println!("装好并信任后，用 Safari 打开 https://{ip}:{port}/m/ 能打开「发到叮咪」页面、没有证书警告就对了。");
    Ok(())
}
