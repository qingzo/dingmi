// 用手机帮新电脑加入叮咪同步（需求-1.x 第 34.3 节）：手机上输电脑显示的 8 位数字，服务器给那台电脑的临时公钥；
// 两边各显示 4 个字（临时公钥的 SHA-256 前 4 个字节各挑一个字，和电脑端 crypto.rs 的 phone_words 一模一样），
// 对得上才把空间密钥加密给它（nacl.box，和发到收件箱一样的格式）。服务器掉包公钥的话 4 个字就对不上
import { sealInbox, toB64 } from "./crypto";

const WORDS = "山水云月日星风雨雪花草木林竹松梅兰菊桃李杏梨橙瓜豆米茶酒糖盐油灯火船车桥门窗屋墙路街城村田河湖海江岛沙石金银铜铁玉珠宝琴书画笔纸墨刀弓箭旗鼓钟铃伞扇帽鞋衣袜被枕床桌椅杯碗盘勺锅壶瓶箱包球鸟鱼虫蛇龙虎牛羊马猪狗猫兔鼠鸡鸭鹅熊鹿狼狐猴象蜂蝶燕鹰鹤雀蛙龟虾蟹贝春夏秋冬早晚东西南北左右红黄蓝绿白黑紫青粉灰甜酸苦辣香暖冷热光影声色心手头眼耳口鼻牙脚背肩腰泉井岩峰谷坡洞港湾溪潭池雷电霜露雾虹冰土泥砖瓦钉线针绳网锁镜尺秤笛箫琵琶歌舞诗词剑盾帆舟塔亭楼阁庙宫园桑麻棉丝绸布葱蒜姜椒蘑菇莲荷柳杨枫桂柏杉橘柚枣栗核蕉莓葡萄饼面汤";

/** 两边核对的 4 个字 */
export async function phoneWords(key: Uint8Array): Promise<string> {
  const h = new Uint8Array(await crypto.subtle.digest("SHA-256", key as BufferSource));
  return [...h.slice(0, 4)].map((b) => WORDS[b]).join("");
}

/** 把空间密钥加密给那台电脑（标准 base64，服务器原样转给电脑） */
export const sealKey = (computerKey: Uint8Array, spaceKey: Uint8Array) => toB64(sealInbox(computerKey, spaceKey));

/** 自建服务器：服务器生成的配对码（不带空间密钥）后面接上空间密钥（base64url、不带 =，和 pair.rs 一样） */
export const withSpaceKey = (code: string, spaceKey: Uint8Array) =>
  `${code}/${toB64(spaceKey).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "")}`;

/** 8 位数字：只留数字 */
export const cleanCode = (s: string) => s.replace(/\D/g, "").slice(0, 8);
