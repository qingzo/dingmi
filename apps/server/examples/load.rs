//! 压力测试：在本机起一个 dingmi-server，模拟很多台电脑同时连着、平时上传、新用户第一次同步、每天备份，
//! 量每一步多久，看 SQLite 和服务器哪里先撑不住。
//!
//! 用法：cargo run --release -p dingmi-server --example load -- [电脑台数，默认 1000] [平时那段跑几秒，默认 60]
//!
//! 「响应」那一行是一条单独的连接每 200 毫秒发一次 Ping：别的请求把数据库占住时，它会跟着变慢，
//! 所以看它的最大值就知道服务有没有被卡住。服务器和 dingmi-server run 一样只用 2 个线程；
//! 模拟的电脑和服务器在同一台机器上，会抢 CPU，数字只是参考。

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD as B64};
use base64::Engine;
use dingmi_server::net::{hash_token, Server};
use dingmi_sync::{connect, generate_keypair, Hello, Msg, NoiseConn, Record, VERSION};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;

type Conn = NoiseConn<TcpStream>;

#[derive(Default)]
struct Stat {
    ms: Mutex<Vec<f64>>,
    errors: AtomicU64,
}

impl Stat {
    fn add(&self, t: Instant) {
        self.ms.lock().unwrap().push(t.elapsed().as_secs_f64() * 1000.0);
    }

    fn report(&self, name: &str) {
        let mut v = self.ms.lock().unwrap().clone();
        let err = self.errors.load(Ordering::Relaxed);
        if v.is_empty() {
            return println!("  {name:<14} 没有数据  出错 {err}");
        }
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let p = |q: f64| v[((v.len() as f64 - 1.0) * q) as usize];
        println!(
            "  {name:<14} {:>6} 次  中位 {:>7.1} ms  95% {:>7.1} ms  99% {:>7.1} ms  最慢 {:>8.1} ms  出错 {err}",
            v.len(),
            p(0.5),
            p(0.95),
            p(0.99),
            v[v.len() - 1]
        );
    }

    fn clear(&self) {
        self.ms.lock().unwrap().clear();
        self.errors.store(0, Ordering::Relaxed);
    }
}

async fn dial(addr: SocketAddr, key: &[u8], spub: &[u8], invite: Option<String>) -> Result<Conn, String> {
    let s = TcpStream::connect(addr).await.map_err(|e| e.to_string())?;
    let hello = Hello { version: VERSION, name: "压测电脑".into(), invite, ..Default::default() };
    let (c, _) = tokio::time::timeout(Duration::from_secs(60), connect(s, key, spub, &hello))
        .await
        .map_err(|_| "握手超时".to_string())?
        .map_err(|e| e.to_string())?;
    Ok(c)
}

async fn ask(c: &mut Conn, m: &Msg) -> Result<Msg, String> {
    c.send(m).await.map_err(|e| e.to_string())?;
    match tokio::time::timeout(Duration::from_secs(120), c.recv::<Msg>()).await {
        Ok(Ok(Msg::Error { msg })) => Err(msg),
        Ok(Ok(r)) => Ok(r),
        _ => Err("连接断了".into()),
    }
}

/// 一批记录：每条 800 字节（差不多是一条带几行字的提醒加密后的大小）
fn batch(dev: usize, from: usize, n: usize) -> Msg {
    let items = (from..from + n)
        .map(|i| Record { uid: format!("d{dev}-{i}"), ver: "0001".into(), seq: 0, blob: STANDARD.encode([7u8; 800]) })
        .collect();
    Msg::Push { items }
}

struct Ctx {
    addr: SocketAddr,
    spub: Vec<u8>,
    keys: Vec<Vec<u8>>,
    stop: AtomicBool,
    probe: Stat,
}

/// 单独一条连接，每 200 毫秒 Ping 一次，量服务有没有被卡住
async fn probe(ctx: Arc<Ctx>) {
    let Ok(mut c) = dial(ctx.addr, &ctx.keys[0], &ctx.spub, None).await else { return };
    while !ctx.stop.load(Ordering::Relaxed) {
        let t = Instant::now();
        match ask(&mut c, &Msg::Ping).await {
            Ok(_) => ctx.probe.add(t),
            Err(_) => {
                ctx.probe.errors.fetch_add(1, Ordering::Relaxed);
                match dial(ctx.addr, &ctx.keys[0], &ctx.spub, None).await {
                    Ok(n) => c = n,
                    Err(_) => return,
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let n: usize = args.first().and_then(|a| a.parse().ok()).unwrap_or(1000);
    let secs: u64 = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(60);

    let dir = std::env::temp_dir().join(format!("dingmi-load-{}", rand::random::<u32>()));
    dingmi_server::init(&dir, "127.0.0.1".parse().unwrap(), 7788).unwrap();
    // 服务器单独一个运行时，和 dingmi-server run 一样只用 2 个线程；模拟的电脑在另一个运行时里，不和它抢线程
    let (tx, rx) = std::sync::mpsc::channel();
    let d = dir.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        rt.block_on(async {
            let server = Server::load(&d).unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            tx.send((server.clone(), listener.local_addr().unwrap())).unwrap();
            server.serve(listener).await;
        });
    });
    let (server, addr) = rx.recv().unwrap();
    let spub = B64.decode(server.store.kv_get("noise_public").unwrap()).unwrap();
    println!("{n} 台电脑（两台一个空间），数据目录 {}", dir.display());

    // ---------- 1. 配对：每台电脑带一个邀请码第一次连上 ----------
    let mut keys = Vec::with_capacity(n);
    let mut invites = Vec::with_capacity(n);
    for i in 0..n {
        let space = format!("space{}", i / 2);
        if i % 2 == 0 {
            server.store.create_space(&space).unwrap();
        }
        let tok = format!("tok{i}-{}", rand::random::<u64>());
        server.store.add_invite(&hash_token(&tok), &space, "2999-01-01 00:00:00").unwrap();
        keys.push(generate_keypair().unwrap().0);
        invites.push(tok);
    }
    let ctx = Arc::new(Ctx { addr, spub, keys, stop: AtomicBool::new(false), probe: Stat::default() });
    let pair = Arc::new(Stat::default());
    let gate = Arc::new(Semaphore::new(200));
    let t0 = Instant::now();
    let mut tasks = vec![];
    for (i, tok) in invites.into_iter().enumerate() {
        let (ctx, pair, gate) = (ctx.clone(), pair.clone(), gate.clone());
        tasks.push(tokio::spawn(async move {
            let _g = gate.acquire().await.unwrap();
            let t = Instant::now();
            match dial(ctx.addr, &ctx.keys[i], &ctx.spub, Some(tok)).await {
                Ok(c) => {
                    pair.add(t);
                    Some(c)
                }
                Err(_) => {
                    pair.errors.fetch_add(1, Ordering::Relaxed);
                    None
                }
            }
        }));
    }
    let mut conns = vec![];
    for t in tasks {
        conns.push(t.await.unwrap());
    }
    println!("\n1. 配对 {n} 台（同时最多 200 台在握手），一共 {:.1} 秒", t0.elapsed().as_secs_f64());
    pair.report("配对");

    // ---------- 2. 平时：都连着等消息；每台平均 30 秒上传一次 5 条；同空间的另一台被叫醒去拉 ----------
    tokio::spawn(probe(ctx.clone()));
    let (push, pull) = (Arc::new(Stat::default()), Arc::new(Stat::default()));
    let mut idle = vec![];
    for (i, c) in conns.into_iter().enumerate() {
        let Some(mut c) = c else { continue };
        let (wctx, pull) = (ctx.clone(), pull.clone());
        // 等消息的那条连接：被叫醒就拉
        idle.push(tokio::spawn(async move {
            let mut since = 0;
            while !wctx.stop.load(Ordering::Relaxed) {
                match ask(&mut c, &Msg::Wait).await {
                    Ok(Msg::Changed) => loop {
                        let t = Instant::now();
                        match ask(&mut c, &Msg::Pull { since }).await {
                            Ok(Msg::Records { items, more }) => {
                                pull.add(t);
                                since = items.last().map(|r| r.seq).unwrap_or(since);
                                if !more {
                                    break;
                                }
                            }
                            _ => {
                                pull.errors.fetch_add(1, Ordering::Relaxed);
                                break;
                            }
                        }
                    },
                    Ok(_) => {}
                    Err(_) => return,
                }
            }
        }));
        // 上传：和电脑端一样另开一条连接，传完就关
        let (pctx, push) = (ctx.clone(), push.clone());
        tokio::spawn(async move {
            let ctx = pctx;
            let mut k = 0;
            while !ctx.stop.load(Ordering::Relaxed) {
                tokio::time::sleep(Duration::from_millis(rand::random::<u64>() % 60_000)).await;
                if ctx.stop.load(Ordering::Relaxed) {
                    break;
                }
                let t = Instant::now();
                let r = match dial(ctx.addr, &ctx.keys[i], &ctx.spub, None).await {
                    Ok(mut c) => ask(&mut c, &batch(i, k, 5)).await.map(|_| ()),
                    Err(e) => Err(e),
                };
                match r {
                    Ok(()) => push.add(t),
                    Err(_) => {
                        push.errors.fetch_add(1, Ordering::Relaxed);
                    }
                }
                k += 5;
            }
        });
    }
    tokio::time::sleep(Duration::from_secs(secs)).await;
    println!("\n2. 平时 {secs} 秒：{n} 台都连着，每台平均 30 秒上传 5 条（连接 + 握手 + 上传），同空间的另一台被叫醒去拉");
    push.report("上传 5 条");
    pull.report("拉取");
    ctx.probe.report("响应（Ping）");
    ctx.probe.clear();

    // ---------- 3. 新用户第一次同步：20 台同时各传 2000 条，每批 100 条（电脑端就是这样分批的） ----------
    let first = Arc::new(Stat::default());
    let t0 = Instant::now();
    let mut tasks = vec![];
    for j in 0..20usize.min(n) {
        let (ctx, first) = (ctx.clone(), first.clone());
        tasks.push(tokio::spawn(async move {
            let i = j * 2 % ctx.keys.len();
            let Ok(mut c) = dial(ctx.addr, &ctx.keys[i], &ctx.spub, None).await else { return };
            for b in 0..20 {
                let t = Instant::now();
                match ask(&mut c, &batch(i, 1_000_000 + b * 100, 100)).await {
                    Ok(_) => first.add(t),
                    Err(_) => {
                        first.errors.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        }));
    }
    for t in tasks {
        let _ = t.await;
    }
    println!("\n3. 20 台同时第一次同步，各传 2000 条（每批 100 条），一共 {:.1} 秒", t0.elapsed().as_secs_f64());
    first.report("一批 100 条");
    ctx.probe.report("响应（Ping）");
    ctx.probe.clear();
    push.clear();
    pull.clear();

    // ---------- 4. 每天的备份：平时的上传照常在跑 ----------
    let db = std::fs::metadata(dir.join(dingmi_server::store::DB_FILE)).map(|m| m.len()).unwrap_or(0);
    let t0 = Instant::now();
    let s2 = server.clone();
    let d2 = dir.clone();
    let r = tokio::task::spawn_blocking(move || s2.store.backup(&d2, "2000-01-01", 14)).await.unwrap();
    let took = t0.elapsed();
    tokio::time::sleep(Duration::from_secs(2)).await;
    println!("\n4. 备份数据库（{:.1} MB）用了 {:.2} 秒{}", db as f64 / 1048576.0, took.as_secs_f64(), if r.is_err() { "，失败了" } else { "" });
    ctx.probe.report("响应（Ping）");
    push.report("上传 5 条");

    ctx.stop.store(true, Ordering::Relaxed);
    let _ = std::fs::remove_dir_all(&dir);
}
