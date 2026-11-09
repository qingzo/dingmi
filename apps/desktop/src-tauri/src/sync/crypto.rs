//! 同步用的加密。设计见 docs/方案/同步方案.md 第 5.1 节。
//!
//! 真正加密数据的是一把随机的**主密钥**。服务器上存两个锁着主密钥的「盒子」：
//! - 盒子 1：用同步密码（Argon2id 换算）和**空间密钥**一起锁着，平时用。空间密钥只在设备上，服务器没有，
//!   所以服务器的数据库被偷了，密码再简单也试不出来；网盘文件夹方式没有空间密钥，只用同步密码；
//! - 盒子 2：用**恢复码**锁着，里面还放着空间密钥。电脑全没了、忘了同步密码时，凭恢复码找回。
//!
//! 记录、图片都用主密钥加密（XChaCha20-Poly1305）。
//!
//! 手机收件箱用另一对密钥（X25519），私钥由主密钥派生，每台输过同步密码的电脑都能算出同一把；
//! 手机网页里只放公钥，用 tweetnacl 的 nacl.box 加密，所以手机只能往里写、不能读。

use argon2::{Algorithm, Argon2, Params, Version};
use base64::engine::general_purpose::STANDARD as B64;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64URL;
use base64::Engine;
use chacha20poly1305::aead::rand_core::RngCore;
use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use crypto_box::aead::AeadInPlace;
use crypto_box::{PublicKey, SalsaBox, SecretKey};
use hkdf::Hkdf;
use sha2::Sha256;

/// 换算参数写在 salt 前面，以后改参数时旧空间还能认出来
const KDF: &str = "argon2id$v=19$m=65536,t=3,p=1";
const NONCE_LEN: usize = 24;
/// 恢复码的字母表：去掉了容易看错的 0、1、I、O，正好 32 个，一个字符 5 位
const RECOVERY_ALPHABET: &[u8; 32] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";

/// 主密钥：记录和图片都用它加密
pub type DataKey = [u8; 32];
/// 空间密钥：只在设备上，和同步密码一起锁盒子 1
pub type SpaceKey = [u8; 32];
/// 恢复码本身：20 字节（160 位），写出来 32 个字符
pub type RecoveryCode = [u8; 20];

fn random<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    OsRng.fill_bytes(&mut b);
    b
}

fn hkdf(ikm: &[u8], info: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    Hkdf::<Sha256>::new(None, ikm).expand(info.as_bytes(), &mut out).expect("32 字节不会超长");
    out
}

fn derive_raw(password: &str, salt: &[u8]) -> Result<[u8; 32], String> {
    let params = Params::new(65536, 3, 1, Some(32)).map_err(|e| e.to_string())?;
    let mut key = [0u8; 32];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|e| e.to_string())?;
    Ok(key)
}

/// 用服务器上存的 salt 和同步密码算出密码钥匙（要零点几秒，故意慢）
pub fn derive(password: &str, salt: &str) -> Result<[u8; 32], String> {
    let (kdf, s) = salt.rsplit_once('$').ok_or("服务器上的 salt 格式不对")?;
    if kdf != KDF {
        return Err("这个空间用的加密参数比这个版本的叮咪新，请先更新叮咪".into());
    }
    derive_raw(password, &B64.decode(s).map_err(|_| "服务器上的 salt 格式不对")?)
}

/// 锁 1：密码钥匙和空间密钥合起来
fn lock_key(password_key: &[u8; 32], space_key: Option<&SpaceKey>) -> [u8; 32] {
    let mut ikm = password_key.to_vec();
    if let Some(k) = space_key {
        ikm.extend_from_slice(k);
    }
    hkdf(&ikm, "dingmi-lock-v1")
}

/// 用同步密码给主密钥换一把锁：回新的 (salt, 盒子 1)。建空间、换同步密码时用
pub fn lock(password: &str, master: &DataKey, space_key: Option<&SpaceKey>) -> Result<(String, String), String> {
    let s: [u8; 16] = random();
    let lock = lock_key(&derive_raw(password, &s)?, space_key);
    Ok((format!("{KDF}${}", B64.encode(s)), B64.encode(seal_with(&lock, b"dingmi-box1-v1", master))))
}

/// 用同步密码打开盒子 1，拿到主密钥
pub fn unlock(password: &str, salt: &str, box1: &str, space_key: Option<&SpaceKey>) -> Result<DataKey, String> {
    let lock = lock_key(&derive(password, salt)?, space_key);
    B64.decode(box1)
        .ok()
        .and_then(|b| open_with(&lock, b"dingmi-box1-v1", &b))
        .and_then(|m| DataKey::try_from(m).ok())
        .ok_or_else(|| "同步密码不对".into())
}

/// 恢复码写出来给人看：DMR-XXXX-XXXX-…（8 组，每组 4 个字符）
pub fn recovery_text(code: &RecoveryCode) -> String {
    let mut chars = Vec::with_capacity(32);
    let (mut acc, mut bits) = (0u32, 0);
    for &b in code {
        acc = (acc << 8) | b as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            chars.push(RECOVERY_ALPHABET[((acc >> bits) & 31) as usize] as char);
        }
    }
    let groups: Vec<String> = chars.chunks(4).map(|c| c.iter().collect()).collect();
    format!("DMR-{}", groups.join("-"))
}

/// 读用户输入的恢复码：不分大小写，空格、横线、开头的 DMR 有没有都行
pub fn parse_recovery(text: &str) -> Result<RecoveryCode, String> {
    const BAD: &str = "恢复码不对，请对照保存的恢复码再输一次";
    let s: String = text.to_uppercase().chars().filter(|c| !c.is_whitespace() && *c != '-').collect();
    let s = s.strip_prefix("DMR").unwrap_or(&s);
    if s.len() != 32 {
        return Err(BAD.into());
    }
    let mut out = Vec::with_capacity(20);
    let (mut acc, mut bits) = (0u32, 0);
    for c in s.bytes() {
        let v = RECOVERY_ALPHABET.iter().position(|&a| a == c).ok_or(BAD)? as u32;
        acc = (acc << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    RecoveryCode::try_from(out).map_err(|_| BAD.into())
}

/// 恢复凭证：没有电脑能生成配对码时，凭它在服务器上登记新电脑。服务器只存它的哈希
pub fn recovery_token(code: &RecoveryCode) -> String {
    B64URL.encode(hkdf(code, "dingmi-recovery-login-v1"))
}

/// 盒子 2：用恢复码锁着主密钥和空间密钥
pub fn seal_box2(code: &RecoveryCode, master: &DataKey, space_key: Option<&SpaceKey>) -> String {
    let mut plain = master.to_vec();
    if let Some(k) = space_key {
        plain.extend_from_slice(k);
    }
    B64.encode(seal_with(&hkdf(code, "dingmi-recovery-key-v1"), b"dingmi-box2-v1", &plain))
}

/// 用恢复码打开盒子 2：回 (主密钥, 空间密钥)
pub fn open_box2(code: &RecoveryCode, box2: &str) -> Result<(DataKey, Option<SpaceKey>), String> {
    let plain = B64
        .decode(box2)
        .ok()
        .and_then(|b| open_with(&hkdf(code, "dingmi-recovery-key-v1"), b"dingmi-box2-v1", &b))
        .ok_or("恢复码不对，或者已经换过新的了")?;
    match plain.len() {
        32 => Ok((plain.try_into().unwrap(), None)),
        64 => Ok((plain[..32].try_into().unwrap(), Some(plain[32..].try_into().unwrap()))),
        _ => Err("服务器上的数据不对".into()),
    }
}

/// 第一台设备建空间时生成的全部东西
pub struct NewSpace {
    pub master: DataKey,
    pub space_key: Option<SpaceKey>,
    pub recovery: RecoveryCode,
    pub salt: String,
    pub box1: String,
    pub box2: String,
}

/// 建空间：随机生成主密钥、恢复码；服务器方式再加一把空间密钥
pub fn new_space(password: &str, with_space_key: bool) -> Result<NewSpace, String> {
    let master: DataKey = random();
    let space_key: Option<SpaceKey> = with_space_key.then(random);
    let recovery: RecoveryCode = random();
    let (salt, box1) = lock(password, &master, space_key.as_ref())?;
    let box2 = seal_box2(&recovery, &master, space_key.as_ref());
    Ok(NewSpace { master, space_key, recovery, salt, box1, box2 })
}

/// 换一个恢复码：回 (新恢复码, 新的盒子 2)，旧的恢复码打不开新盒子
pub fn new_recovery(master: &DataKey, space_key: Option<&SpaceKey>) -> (RecoveryCode, String) {
    let code: RecoveryCode = random();
    let box2 = seal_box2(&code, master, space_key);
    (code, box2)
}

/// 加密：输出 nonce（24 字节）+ 密文。`aad` 是一起认证、但不加密的附加数据（记录的编号和版本），
/// 服务器把密文挪到别的记录上就解不开
pub fn seal_with(key: &DataKey, aad: &[u8], plain: &[u8]) -> Vec<u8> {
    let cipher = XChaCha20Poly1305::new(Key::from_slice(key));
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let mut out = nonce.to_vec();
    out.extend(cipher.encrypt(&nonce, Payload { msg: plain, aad }).expect("加密内存数据不会失败"));
    out
}

/// 解密 `seal_with` 的输出；密钥、附加数据不对或数据被改过都返回 None
pub fn open_with(key: &DataKey, aad: &[u8], data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < NONCE_LEN {
        return None;
    }
    let (nonce, ct) = data.split_at(NONCE_LEN);
    XChaCha20Poly1305::new(Key::from_slice(key)).decrypt(XNonce::from_slice(nonce), Payload { msg: ct, aad }).ok()
}

fn inbox_secret(key: &DataKey) -> SecretKey {
    SecretKey::from(hkdf(key, "dingmi-inbox-v1"))
}

/// 收件箱公钥，写进手机网页的网址里
pub fn inbox_public(key: &DataKey) -> [u8; 32] {
    inbox_secret(key).public_key().to_bytes()
}

/// 解开手机发来的一条
pub fn open_inbox(key: &DataKey, data: &[u8]) -> Option<Vec<u8>> {
    open_box(&inbox_secret(key), data)
}

/// 格式：手机的临时公钥(32) + nonce(24) + 认证码(16) + 密文，和 tweetnacl 的 nacl.box 一致
fn open_box(secret: &SecretKey, data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 72 {
        return None;
    }
    let (epk, rest) = data.split_at(32);
    let (nonce, rest) = rest.split_at(24);
    let (tag, ct) = rest.split_at(16);
    let epk = PublicKey::from(<[u8; 32]>::try_from(epk).ok()?);
    let mut buf = ct.to_vec();
    SalsaBox::new(&epk, secret)
        .decrypt_in_place_detached(nonce.into(), b"", &mut buf, tag.into())
        .ok()?;
    Some(buf)
}

// ---------- 新电脑方便加入（需求-1.x 第 34 节，同步方案.md 第 5.1 节） ----------

/// 兑换码加入要的同步密码至少几位（作者定 10 位）
pub const JOIN_MIN_CHARS: usize = 10;

/// 同步密码够长、不太常见，才上传盒子 3（开兑换码加入）：服务器的数据库被偷走时，靠它挡住离线猜
pub fn join_allowed(password: &str) -> bool {
    password.chars().count() >= JOIN_MIN_CHARS && !common_password(password)
}

/// 太常见的：同一个字符、连着的数字字母（顺着倒着）、常见的那几个
fn common_password(password: &str) -> bool {
    const COMMON: [&str; 16] = [
        "password123", "password1234", "1qaz2wsx3edc", "1q2w3e4r5t", "1q2w3e4r5t6y", "qwertyuiop", "qwerty123456",
        "a1234567890", "a123456789", "abc1234567", "abcd123456", "iloveyou520", "woaini1314", "woaini520520", "asdfghjkl;", "zxcvbnm123",
    ];
    let p = password.to_lowercase();
    let b = p.as_bytes();
    let same = b.windows(2).all(|w| w[0] == w[1]);
    let up = b.windows(2).all(|w| w[1] == w[0].wrapping_add(1));
    let down = b.windows(2).all(|w| w[0] == w[1].wrapping_add(1));
    same || up || down || COMMON.contains(&p.as_str()) || "1234567890".contains(p.as_str()) || "0987654321".contains(p.as_str())
}

/// 加入核对值：电脑带给服务器，服务器只存哈希
fn join_proof_of(password_key: &[u8; 32]) -> String {
    B64URL.encode(hkdf(password_key, "dingmi-join-login-v1"))
}

fn box3_key(password_key: &[u8; 32]) -> [u8; 32] {
    hkdf(password_key, "dingmi-box3-key-v1")
}

/// 设同步密码、换同步密码、补开兑换码加入时：回 (盒子 3, 加入核对值)。salt 是这次的（盒子 1 用的那个）
pub fn join_lock(password: &str, salt: &str, space_key: &SpaceKey) -> Result<(String, String), String> {
    let p = derive(password, salt)?;
    Ok((B64.encode(seal_with(&box3_key(&p), b"dingmi-box3-v1", space_key)), join_proof_of(&p)))
}

/// 兑换码加入的新电脑：用同步密码和服务器给的 salt 算出 (加入核对值, 密码钥匙)；密码钥匙留着开盒子 3
pub fn join_proof(password: &str, salt: &str) -> Result<(String, [u8; 32]), String> {
    let p = derive(password, salt)?;
    Ok((join_proof_of(&p), p))
}

/// 用密码钥匙打开盒子 3，拿到空间密钥
pub fn open_box3(password_key: &[u8; 32], box3: &str) -> Result<SpaceKey, String> {
    B64.decode(box3)
        .ok()
        .and_then(|b| open_with(&box3_key(password_key), b"dingmi-box3-v1", &b))
        .and_then(|k| SpaceKey::try_from(k).ok())
        .ok_or_else(|| "同步密码不对".into())
}

/// 用手机加入时两边核对的字：电脑临时公钥的 SHA-256 前 4 个字节，各挑一个字（手机网页 apps/mobile/join.ts 一模一样）
const PHONE_WORDS: &str = "山水云月日星风雨雪花草木林竹松梅兰菊桃李杏梨橙瓜豆米茶酒糖盐油灯火船车桥门窗屋墙路街城村田河湖海江岛沙石金银铜铁玉珠宝琴书画笔纸墨刀弓箭旗鼓钟铃伞扇帽鞋衣袜被枕床桌椅杯碗盘勺锅壶瓶箱包球鸟鱼虫蛇龙虎牛羊马猪狗猫兔鼠鸡鸭鹅熊鹿狼狐猴象蜂蝶燕鹰鹤雀蛙龟虾蟹贝春夏秋冬早晚东西南北左右红黄蓝绿白黑紫青粉灰甜酸苦辣香暖冷热光影声色心手头眼耳口鼻牙脚背肩腰泉井岩峰谷坡洞港湾溪潭池雷电霜露雾虹冰土泥砖瓦钉线针绳网锁镜尺秤笛箫琵琶歌舞诗词剑盾帆舟塔亭楼阁庙宫园桑麻棉丝绸布葱蒜姜椒蘑菇莲荷柳杨枫桂柏杉橘柚枣栗核蕉莓葡萄饼面汤";

pub fn phone_words(public: &[u8]) -> String {
    use sha2::Digest;
    let words: Vec<char> = PHONE_WORDS.chars().collect();
    sha2::Sha256::digest(public)[..4].iter().map(|&b| words[b as usize]).collect()
}

/// 用手机加入：这台电脑这次的临时钥匙 (私钥, 公钥)
pub fn phone_keypair() -> ([u8; 32], [u8; 32]) {
    let s = SecretKey::from(random::<32>());
    (s.to_bytes(), s.public_key().to_bytes())
}

/// 解开手机加密给这台电脑的空间密钥（nacl.box 的格式，和收件箱一样）
pub fn open_phone(secret: &[u8; 32], sealed: &[u8]) -> Result<SpaceKey, String> {
    open_box(&SecretKey::from(*secret), sealed)
        .and_then(|k| SpaceKey::try_from(k).ok())
        .ok_or_else(|| "手机发来的钥匙解不开，再试一次".into())
}

/// 测试用：像手机网页那样（或者拿到收件箱公钥的任何人）往收件箱里封一条
#[cfg(test)]
pub fn seal_inbox(public: &[u8; 32], plain: &[u8]) -> Vec<u8> {
    let eph = SecretKey::from([7u8; 32]);
    let nonce = [1u8; 24];
    let mut buf = plain.to_vec();
    let tag = SalsaBox::new(&PublicKey::from(*public), &eph).encrypt_in_place_detached((&nonce).into(), b"", &mut buf).unwrap();
    let mut out = eph.public_key().to_bytes().to_vec();
    out.extend(nonce);
    out.extend(tag);
    out.extend(buf);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlock_with_password_and_space_key() {
        let ns = new_space("correct horse", true).unwrap();
        assert!(ns.salt.starts_with("argon2id$v=19$m=65536,t=3,p=1$"));
        let k = ns.space_key.unwrap();
        // 另一台电脑：同一个同步密码和空间密钥打开盒子 1，拿到同一把主密钥
        assert_eq!(unlock("correct horse", &ns.salt, &ns.box1, Some(&k)).unwrap(), ns.master);
        assert_eq!(unlock("correct horsf", &ns.salt, &ns.box1, Some(&k)).unwrap_err(), "同步密码不对");
        // 只有密码、没有空间密钥（比如偷了服务器数据库的人）打不开
        assert!(unlock("correct horse", &ns.salt, &ns.box1, None).is_err());
        assert!(unlock("correct horse", &ns.salt, &ns.box1, Some(&[1; 32])).is_err());
        // 换同步密码：主密钥不变
        let (salt2, box1b) = lock("new password", &ns.master, Some(&k)).unwrap();
        assert_ne!(salt2, ns.salt);
        assert_eq!(unlock("new password", &salt2, &box1b, Some(&k)).unwrap(), ns.master);
    }

    /// 兑换码加入：盒子 3 用同步密码打开拿到空间密钥，再和平时一样打开盒子 1；核对值只跟密码和 salt 有关
    #[test]
    fn box3_and_join_proof() {
        let ns = new_space("correct horse battery", true).unwrap();
        let k = ns.space_key.unwrap();
        let (box3, proof) = join_lock("correct horse battery", &ns.salt, &k).unwrap();
        let (proof2, p) = join_proof("correct horse battery", &ns.salt).unwrap();
        assert_eq!(proof, proof2);
        assert_eq!(open_box3(&p, &box3).unwrap(), k);
        assert_eq!(unlock("correct horse battery", &ns.salt, &ns.box1, Some(&k)).unwrap(), ns.master);
        let (bad, p2) = join_proof("correct horse batterz", &ns.salt).unwrap();
        assert_ne!(bad, proof);
        assert!(open_box3(&p2, &box3).is_err());
        // 换了 salt（换同步密码），核对值跟着变
        let (salt2, _) = lock("correct horse battery", &ns.master, Some(&k)).unwrap();
        assert_ne!(join_proof("correct horse battery", &salt2).unwrap().0, proof);
    }

    #[test]
    fn join_needs_long_uncommon_password() {
        assert!(join_allowed("我的同步密码很长很长了"));
        assert!(join_allowed("dingmi-2026!"));
        for p in ["short-pw", "123456789", "1234567890", "0987654321", "aaaaaaaaaaaa", "abcdefghijk", "QwertyUiop", "password123"] {
            assert!(!join_allowed(p), "{p}");
        }
    }

    /// 用手机加入：手机（任何拿到电脑临时公钥的人）加密的空间密钥，电脑用临时私钥解开；4 个字固定、跟手机算的一样
    #[test]
    fn phone_seal_and_words() {
        let (secret, public) = phone_keypair();
        let sealed = seal_inbox(&public, &[9u8; 32]);
        assert_eq!(open_phone(&secret, &sealed).unwrap(), [9u8; 32]);
        assert!(open_phone(&phone_keypair().0, &sealed).is_err());
        assert_eq!(PHONE_WORDS.chars().count(), 256);
        let w = phone_words(&[7u8; 32]);
        assert_eq!(w.chars().count(), 4);
        // 和手机网页（apps/mobile/join.test.ts）用同一组数据核对
        assert_eq!(w, "鞋峰鹿绿");
    }

    #[test]
    fn folder_space_has_no_space_key() {
        let ns = new_space("correct horse", false).unwrap();
        assert!(ns.space_key.is_none());
        assert_eq!(unlock("correct horse", &ns.salt, &ns.box1, None).unwrap(), ns.master);
        assert_eq!(open_box2(&ns.recovery, &ns.box2).unwrap(), (ns.master, None));
    }

    #[test]
    fn recovery_code_opens_box2() {
        let ns = new_space("x", true).unwrap();
        // 恢复码单独就能拿到主密钥和空间密钥，不用同步密码
        assert_eq!(open_box2(&ns.recovery, &ns.box2).unwrap(), (ns.master, ns.space_key));
        let typed = recovery_text(&ns.recovery).to_lowercase().replace('-', " ");
        assert_eq!(open_box2(&parse_recovery(&typed).unwrap(), &ns.box2).unwrap().0, ns.master);
        // 换了恢复码以后，旧的打不开新盒子
        let (code2, box2b) = new_recovery(&ns.master, ns.space_key.as_ref());
        assert_eq!(open_box2(&code2, &box2b).unwrap().0, ns.master);
        assert!(open_box2(&ns.recovery, &box2b).is_err());
        assert_ne!(recovery_token(&code2), recovery_token(&ns.recovery));
    }

    #[test]
    fn recovery_text_roundtrip() {
        let code: RecoveryCode = std::array::from_fn(|i| (i * 37 + 11) as u8);
        let text = recovery_text(&code);
        assert_eq!(text.len(), 4 + 32 + 7);
        assert!(text.starts_with("DMR-") && !text[4..].contains(['0', '1', 'I', 'O']));
        assert_eq!(parse_recovery(&text).unwrap(), code);
        assert_eq!(parse_recovery(&format!("  {}\n", text.to_lowercase())).unwrap(), code);
        assert_eq!(parse_recovery(&text[4..].replace('-', "")).unwrap(), code);
        assert_eq!(recovery_text(&[0; 20]), "DMR-2222-2222-2222-2222-2222-2222-2222-2222");
        assert_eq!(recovery_text(&[255; 20]), "DMR-ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZ");
        let typo = format!("DMR-0{}", &text[5..]);
        for bad in ["", "DMR-2222", &text[..text.len() - 1], typo.as_str()] {
            assert!(parse_recovery(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn seal_and_open() {
        let key = [7u8; 32];
        let a = seal_with(&key, b"", "预算 20 万".as_bytes());
        let b = seal_with(&key, b"", "预算 20 万".as_bytes());
        // 每次 nonce 都不一样
        assert_ne!(a, b);
        assert_eq!(open_with(&key, b"", &a).unwrap(), "预算 20 万".as_bytes());
        let mut bad = a.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert!(open_with(&key, b"", &bad).is_none());
        assert!(open_with(&[8u8; 32], b"", &a).is_none());
        assert!(open_with(&key, b"", b"short").is_none());
        // 附加数据对不上（比如密文被挪到别的记录上）就解不开
        let c = seal_with(&key, b"n1|0001", b"x");
        assert_eq!(open_with(&key, b"n1|0001", &c).unwrap(), b"x");
        assert!(open_with(&key, b"n2|0001", &c).is_none());
    }

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    /// 用 tweetnacl 1.0.3 生成的数据：手机网页加密的，电脑这边要能解开
    #[test]
    fn opens_tweetnacl_box() {
        let secret = SecretKey::from(<[u8; 32]>::try_from((1..=32).collect::<Vec<u8>>()).unwrap());
        assert_eq!(
            secret.public_key().to_bytes().to_vec(),
            hex("07a37cbc142093c8b755dc1b10e86cb426374ad16aa853ed0bdfc0b2b86d1c7c")
        );
        let mut data = hex("5869aff450549732cbaaed5e5df9b30a6da31cb0e5742bad5ad4a1a768f1a67b");
        data.extend([7u8; 24]);
        data.extend(hex("77212c9bb7a9063c51737fa8bf26b56730f5dc4fab9bf2edaf1efe65"));
        assert_eq!(open_box(&secret, &data).unwrap(), "嘿，叮咪".as_bytes());
        // 改一个字节就解不开
        let last = data.len() - 1;
        data[last] ^= 1;
        assert!(open_box(&secret, &data).is_none());
    }

    /// 手机版（apps/mobile/crypto.ts，hash-wasm + @noble/ciphers）算出的密钥和加密的记录，电脑这边要一模一样、解得开
    #[test]
    fn opens_mobile_record() {
        let key = derive("password1", "argon2id$v=19$m=65536,t=3,p=1$AQIDBAUGBwgJCgsMDQ4PEA==").unwrap();
        assert_eq!(key.to_vec(), hex("93ba2ccdf7c55f47526151968b8699d05e07bf0edfc3431c342b1f686b325645"));
        let blob = B64
            .decode(
                "H6zf5+s1i97ivWk8jxn6+YSHVCuCaEwTeB4HV4IKnaAMfkaTLtPQ7uoXjdM6592uxtevTqJ8Z3Z7mJkDQtcYQuG/BY1tB3qS2/kbDDoqC5L0jZ398juwgSGyQm8LH8eeUUXwGcjA7qpm+/WvepEmpJd0dyLBOOeV2hO7M/OCittiFJsMiBTgagv0PZCuSIXHCg==",
            )
            .unwrap();
        let aad = b"0192abcd-0000-7000-8000-000000000001|1730000000000-0000-aabbccddeeff";
        let plain = String::from_utf8(open_with(&key, aad, &blob).unwrap()).unwrap();
        assert_eq!(
            plain,
            r#"{"k":"note","d":{"uid":"0192abcd-0000-7000-8000-000000000001","title":"手机记的","body":"汽锅鸡"}}"#
        );
        assert!(open_with(&key, b"x", &blob).is_none());
    }

    /// 手机版（apps/mobile/crypto.ts 的 lockKey + seal，WebCrypto 的 HKDF）锁的盒子 1，电脑这边要打得开：
    /// 两边的锁 1 算法一样。主密钥 [5; 32]，空间密钥 [9; 32]
    #[test]
    fn opens_mobile_box1() {
        let salt = "argon2id$v=19$m=65536,t=3,p=1$AQIDBAUGBwgJCgsMDQ4PEA==";
        let box1 = "c1Pqe6plqEdCem/XlpShIA+hv58HYRQ/zmfdCU2APqlSRm5HrxPivMW2R91+GsHnuinovtCok/50okI2AVo56X9E1A8I2yOU";
        assert_eq!(unlock("password1", salt, box1, Some(&[9; 32])).unwrap(), [5; 32]);
        assert!(unlock("password1", salt, box1, Some(&[8; 32])).is_err());
        assert!(unlock("password2", salt, box1, Some(&[9; 32])).is_err());
    }

    #[test]
    fn inbox_key_same_on_every_device() {
        let key = [3u8; 32];
        assert_eq!(inbox_public(&key), inbox_public(&key));
        assert_ne!(inbox_public(&key), inbox_public(&[4u8; 32]));
    }

    #[test]
    fn rejects_unknown_kdf() {
        assert!(derive("x", "argon2id$v=19$m=1,t=1,p=1$AAAA").is_err());
        assert!(derive("x", "garbage").is_err());
    }
}
