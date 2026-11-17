// 提示音（需求-1.x 第 42.5 节）：内置几种用程序合成，不用音频文件、没有版权问题，安装包也不变大；
// 也能用自己选的声音文件（复制到数据目录里，只在这台电脑上）。提醒卡片和设置里试听都从这里放。
import { convertFileSrc } from "@tauri-apps/api/core";

/** 内置的几种：[id, 名字] */
export const SOUNDS: [string, string][] = [
  ["dingmi", "叮咪"],
  ["bright", "清脆"],
  ["marimba", "木琴"],
  ["soft", "柔和"],
  ["bell", "铃铛"],
  ["alarm", "闹钟"],
];
export const CUSTOM = "custom";

export interface SoundOpt {
  /** 0–100，70 是原来「叮」的音量 */
  volume: number;
  /** 紧急的：同一种声音响一点 */
  loud?: boolean;
  /** 自己选的声音文件（本机路径）；custom 用，没有就用叮咪 */
  file?: string;
}

type Note = { f: number; at: number; len: number; type?: OscillatorType; peak?: number; attack?: number };

/** 每一种是几个音：频率、什么时候开始、多长、波形、相对响度 */
function notes(kind: string, loud: boolean): Note[] {
  switch (kind) {
    case "bright":
      return [1568, 2093, 2637, ...(loud ? [3136] : [])].map((f, i) => ({ f, at: i * 0.09, len: 0.55, type: "triangle", peak: 1.3 }));
    case "marimba":
      // 每个音带一个四倍频，敲下去的那一下更像木头
      return [523.25, 659.25, 783.99, 1046.5, ...(loud ? [1318.5] : [])].flatMap((f, i) => [
        { f, at: i * 0.12, len: 0.45, peak: 1.2 },
        { f: f * 4, at: i * 0.12, len: 0.08, peak: 0.25 },
      ]);
    case "soft":
      return [
        { f: 523.25, at: 0, len: 1.6, peak: 0.8, attack: 0.15 },
        { f: 659.25, at: 0, len: 1.6, peak: 0.6, attack: 0.15 },
        { f: 783.99, at: 0.45, len: 1.5, peak: 0.7, attack: 0.15 },
        ...(loud ? [{ f: 1046.5, at: 0.9, len: 1.4, peak: 0.6, attack: 0.15 }] : []),
      ];
    case "bell":
      // 铃铛：几个不成倍数的泛音，高的散得快；敲两下
      return [0, 0.9, ...(loud ? [1.8] : [])].flatMap((at) =>
        [
          [1, 1, 2.2],
          [2.76, 0.5, 1.4],
          [5.4, 0.25, 0.8],
          [8.93, 0.12, 0.5],
        ].map(([k, peak, len]) => ({ f: 784 * k, at, len, peak })),
      );
    case "alarm": {
      // 闹钟：哔哔哔哔，三组（紧急的四组），一共两三秒
      const out: Note[] = [];
      for (let g = 0; g < (loud ? 4 : 3); g++) for (let b = 0; b < 4; b++) out.push({ f: 988, at: g * 0.75 + b * 0.13, len: 0.08, type: "square", peak: 0.45, attack: 0.005 });
      return out;
    }
    default:
      // 叮咪：原来那一声「叮」
      return [
        { f: 880, at: 0, len: 0.9 },
        { f: 1318.5, at: 0.16, len: 0.9 },
        ...(loud ? [{ f: 1760, at: 0.32, len: 0.9 }] : []),
      ];
  }
}

/** 正在响的，换一个或者停的时候先关掉；by 是哪一次 ring 放的 */
let cur: { ctx?: AudioContext; audio?: HTMLAudioElement; by?: object } = {};

export function stopSound() {
  cur.ctx?.close().catch(() => {});
  if (cur.audio) {
    cur.audio.pause();
    cur.audio.src = "";
  }
  cur = {};
}

/** 合成的放一遍，回大概多长（毫秒） */
function synth(kind: string, o: SoundOpt, by: object): number {
  const loud = !!o.loud;
  const list = notes(kind, loud);
  // 70 是原来的音量；紧急的再响一点
  const base = (loud ? 0.42 : 0.22) * (Math.max(0, Math.min(100, o.volume)) / 70);
  try {
    const ctx = new AudioContext();
    cur = { ctx, by };
    for (const n of list) {
      const osc = ctx.createOscillator();
      const g = ctx.createGain();
      osc.type = n.type ?? "sine";
      osc.frequency.value = n.f;
      const t = ctx.currentTime + n.at;
      const peak = Math.min(0.9, base * (n.peak ?? 1));
      g.gain.setValueAtTime(0, t);
      g.gain.linearRampToValueAtTime(peak, t + (n.attack ?? 0.02));
      g.gain.exponentialRampToValueAtTime(0.001, t + n.len);
      osc.connect(g).connect(ctx.destination);
      osc.start(t);
      osc.stop(t + n.len + 0.1);
    }
    const ms = Math.max(...list.map((n) => n.at + n.len)) * 1000 + 200;
    setTimeout(() => cur.ctx === ctx && stopSound(), ms + 300);
    return ms;
  } catch {
    // 没有声卡也没关系
    return 1000;
  }
}

/**
 * 响一下；loop 时一直响，直到调回来的函数被调用，或者满 1 分钟。
 * 自己选的文件放不出来（别的电脑上没有、文件坏了）就用叮咪。
 */
export function ring(kind: string, o: SoundOpt, loop = false): () => void {
  stopSound();
  const me = {};
  let stopped = false;
  let timer = 0;
  let cap = 0;
  const until = Date.now() + 60_000;
  const stop = () => {
    if (stopped) return;
    stopped = true;
    clearTimeout(timer);
    clearTimeout(cap);
    // 已经换成别的在响了就不管
    if (cur.by === me) stopSound();
  };
  const again = (ms: number) => {
    if (loop && !stopped && Date.now() + ms < until) timer = window.setTimeout(once, ms + 700);
  };
  const fallback = () => !stopped && again(synth("dingmi", o, me));
  const once = () => {
    if (stopped) return;
    if (kind !== CUSTOM || !o.file) return again(synth(kind, o, me));
    const audio = new Audio(convertFileSrc(o.file));
    audio.volume = Math.max(0, Math.min(1, o.volume / 100));
    cur = { audio, by: me };
    audio.onended = () => again(0);
    audio.onerror = fallback;
    audio.play().catch(fallback);
    // 很长的文件也只放 1 分钟
    cap ||= window.setTimeout(stop, Math.max(0, until - Date.now()));
  };
  once();
  return stop;
}
