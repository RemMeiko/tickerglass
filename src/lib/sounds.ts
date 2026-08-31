/**
 * 提示音引擎。
 *
 * # 为什么内置音色是**合成**的而不是打包 mp3/wav
 *
 * 1. **版权**：从网上找的音频，授权状态往往无法确认。给一个要分发的桌面应用
 *    打包来源不明的素材是实际风险，而合成音没有这个问题。
 * 2. **体积**：参考项目带了 117KB + 64KB 两个 wav。合成音是零字节。
 * 3. **可控**：能精确调出"短促、中性、不刺耳"的效果；网上的提示音素材
 *    经常带混响尾巴或音量忽高忽低。
 *
 * 用户想要真实音频时，可以在设置里选自己的文件（`custom:` 前缀），
 * 那条路走 `<audio>` 播放，与这里的合成互不影响。
 *
 * # 音色设计原则
 *
 * 用户明确说了**不要牛叫这种**。所以全部做成中性的界面提示音，
 * 靠"上行/下行"的音高走向区分涨跌，不靠拟声：
 * - 涨：音高**上行**
 * - 跌：音高**下行**
 *
 * 这样不看屏幕也能听出方向，且不会有动物叫声的滑稽感。
 */

export type ToneId =
  | "chime"
  | "drop"
  | "wood"
  | "bell"
  | "tick"
  | "marimba"
  | "none";

export interface ToneMeta {
  id: ToneId;
  label: string;
  /** 一句话描述，显示在下拉项旁边帮用户选 */
  hint: string;
}

/** 内置音色清单。`none` 排最后，它等价于静音。 */
export const TONES: ToneMeta[] = [
  { id: "chime", label: "清铃", hint: "两声清脆的铃音，最通用" },
  { id: "marimba", label: "木琴", hint: "三音琶音，柔和不突兀" },
  { id: "bell", label: "钟磬", hint: "单声长尾，余音较久" },
  { id: "drop", label: "水滴", hint: "短促圆润，适合安静环境" },
  { id: "wood", label: "木鱼", hint: "干脆的敲击声，无余音" },
  { id: "tick", label: "双击", hint: "两下极短的哒声，最不打扰" },
  { id: "none", label: "静音", hint: "只弹卡片，不出声" },
];

export function isToneId(v: string): v is ToneId {
  return TONES.some((t) => t.id === v);
}

/** 自定义音频的配置值前缀。`custom:文件名` */
export const CUSTOM_PREFIX = "custom:";

export function customFileName(value: string): string | null {
  return value.startsWith(CUSTOM_PREFIX)
    ? value.slice(CUSTOM_PREFIX.length)
    : null;
}

// ───────────────────── 合成实现 ─────────────────────

/** 单个正弦/三角音符。所有音色都由它拼出来。 */
function note(
  ctx: AudioContext,
  opts: {
    /** 起始频率 Hz */
    freq: number;
    /** 终止频率；与 freq 不同则做滑音 */
    toFreq?: number;
    /** 相对 ctx.currentTime 的起始偏移（秒） */
    at: number;
    /** 时长（秒） */
    dur: number;
    /** 峰值增益（0~1），已含总音量系数 */
    peak: number;
    type?: OscillatorType;
    /** 起音时间（秒）。越短越"硬"，木鱼类用极短值 */
    attack?: number;
  }
) {
  const t0 = ctx.currentTime + opts.at;
  const osc = ctx.createOscillator();
  const gain = ctx.createGain();
  osc.type = opts.type ?? "sine";
  osc.frequency.setValueAtTime(opts.freq, t0);
  if (opts.toFreq && opts.toFreq !== opts.freq) {
    osc.frequency.exponentialRampToValueAtTime(opts.toFreq, t0 + opts.dur);
  }
  const attack = opts.attack ?? 0.006;
  // exponentialRamp 的目标不能是 0，故用极小值收尾
  gain.gain.setValueAtTime(0.0001, t0);
  gain.gain.exponentialRampToValueAtTime(Math.max(opts.peak, 0.0002), t0 + attack);
  gain.gain.exponentialRampToValueAtTime(0.0001, t0 + opts.dur);
  osc.connect(gain).connect(ctx.destination);
  osc.start(t0);
  osc.stop(t0 + opts.dur + 0.02);
}

/**
 * 播放一个内置音色。
 *
 * @param up true=上涨（音高上行）、false=下跌（下行）。
 *   同一音色的涨跌版本只差音高走向 —— 这样音色是"听感风格"，
 *   方向是"信息"，两者独立，用户换音色不会丢掉方向信息。
 * @param volume 0~1
 */
export function playTone(
  ctx: AudioContext,
  id: ToneId,
  up: boolean,
  volume: number
): void {
  const v = Math.max(0, Math.min(1, volume));
  if (id === "none" || v === 0) return;
  // 峰值压到 0.22 以下：提示音不该比系统音量还冲
  const g = 0.22 * v;

  switch (id) {
    case "chime": {
      // 两声清铃。上涨 C6→E6，下跌 E6→C6
      const a = up ? 1046.5 : 1318.5;
      const b = up ? 1318.5 : 1046.5;
      note(ctx, { freq: a, at: 0, dur: 0.22, peak: g, type: "sine" });
      note(ctx, { freq: b, at: 0.11, dur: 0.3, peak: g * 0.85, type: "sine" });
      break;
    }
    case "marimba": {
      // 三音琶音，木琴质感用三角波 + 快速衰减
      const seq = up ? [659.3, 830.6, 987.8] : [987.8, 830.6, 659.3];
      seq.forEach((f, i) =>
        note(ctx, {
          freq: f,
          at: i * 0.075,
          dur: 0.26,
          peak: g * (1 - i * 0.12),
          type: "triangle",
        })
      );
      break;
    }
    case "bell": {
      // 单声长尾。基音 + 一个五度泛音让它有金属感
      const base = up ? 880 : 587.3;
      note(ctx, { freq: base, at: 0, dur: 0.9, peak: g, type: "sine" });
      note(ctx, {
        freq: base * 1.5,
        at: 0,
        dur: 0.7,
        peak: g * 0.28,
        type: "sine",
      });
      break;
    }
    case "drop": {
      // 水滴：短促滑音，上涨向上滑、下跌向下滑
      note(ctx, {
        freq: up ? 620 : 900,
        toFreq: up ? 1150 : 480,
        at: 0,
        dur: 0.16,
        peak: g,
        type: "sine",
        attack: 0.004,
      });
      break;
    }
    case "wood": {
      // 木鱼：极短起音 + 无余音。方向靠音高高低区分
      note(ctx, {
        freq: up ? 1200 : 760,
        at: 0,
        dur: 0.075,
        peak: g,
        type: "triangle",
        attack: 0.001,
      });
      break;
    }
    case "tick": {
      // 双击：两下极短的哒声，最不打扰
      const f = up ? 1500 : 950;
      note(ctx, { freq: f, at: 0, dur: 0.04, peak: g * 0.8, type: "square", attack: 0.001 });
      note(ctx, { freq: f, at: 0.085, dur: 0.04, peak: g * 0.8, type: "square", attack: 0.001 });
      break;
    }
  }
}

// ───────────────────── 统一播放入口 ─────────────────────

/** 自定义音频的 Blob URL 缓存：避免每次提醒都过一次 IPC 读盘 */
const blobCache = new Map<string, string>();

let sharedCtx: AudioContext | null = null;

/**
 * 懒建 AudioContext。
 *
 * 不在模块加载时建：浏览器/WebView 要求 AudioContext 在用户交互后才能出声，
 * 过早创建会得到一个 suspended 的实例。
 */
function audioCtx(): AudioContext | null {
  try {
    sharedCtx ??= new AudioContext();
    // 被浏览器挂起时尝试恢复（首次交互前创建的实例会是 suspended）
    if (sharedCtx.state === "suspended") void sharedCtx.resume();
    return sharedCtx;
  } catch {
    return null;
  }
}

/**
 * 播放提醒音。**AlertCard 与设置页试听共用这一个函数** ——
 * 分两份实现必然出现"试听没问题但实际提醒时不一样"。
 *
 * @param tone 配置里的音色值：内置 id 或 `custom:文件名`
 * @param up true=上涨
 * @param volume 0~1
 * @param readCustom 读自定义音频字节的函数（由调用方注入 IPC，
 *   这样本模块不依赖 tauri，可在 vitest 里直接测）
 */
export async function playAlertSound(
  tone: string,
  up: boolean,
  volume: number,
  readCustom?: (fileName: string) => Promise<Uint8Array>
): Promise<void> {
  const v = Math.max(0, Math.min(1, volume));
  if (v === 0) return;

  const custom = customFileName(tone);
  if (custom) {
    try {
      let url = blobCache.get(custom);
      if (!url) {
        if (!readCustom) return;
        const bytes = await readCustom(custom);
        url = URL.createObjectURL(new Blob([bytes]));
        blobCache.set(custom, url);
      }
      const el = new Audio(url);
      el.volume = v;
      await el.play();
      return;
    } catch {
      // 自定义音频播不出来（文件被删 / 格式实际不支持）时**退回内置音**，
      // 而不是静默无声 —— 用户设了提示音却完全没反应是最难排查的情况。
      // 退回后仍能听见提醒，只是音色不是他选的那个。
      try {
        const ctx = audioCtx();
        if (ctx) playTone(ctx, "chime", up, v);
      } catch {
        // 连内置音都失败：放弃出声，视觉提醒不受影响
      }
      return;
    }
  }

  // 内置音色。认不出的 id 一律静音而不是抛错 ——
  // 后端 sanitize 只校验格式不校验清单，所以这里必须能容忍陌生 id。
  if (!isToneId(tone)) return;
  const ctx = audioCtx();
  if (ctx) playTone(ctx, tone, up, v);
}

/**
 * 清掉某个自定义音频的缓存。删除或覆盖导入后必须调，
 * 否则会继续播放旧的 Blob（文件已删但声音还在，很费解）。
 */
export function invalidateSoundCache(fileName?: string): void {
  if (fileName) {
    const url = blobCache.get(fileName);
    if (url) URL.revokeObjectURL(url);
    blobCache.delete(fileName);
    return;
  }
  for (const url of blobCache.values()) URL.revokeObjectURL(url);
  blobCache.clear();
}
