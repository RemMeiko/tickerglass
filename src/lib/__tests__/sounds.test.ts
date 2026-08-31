import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  CUSTOM_PREFIX,
  TONES,
  customFileName,
  invalidateSoundCache,
  isToneId,
  playAlertSound,
} from "../sounds";

// vitest 跑在 node 环境（见 vitest.config.ts），没有 AudioContext / Audio /
// URL.createObjectURL。这里造最小替身，只为验证**播放路由**的分支走向 ——
// 不验证音频听起来如何（那是人耳的事）。

/** 记录每次 playTone 实际建了几个 oscillator，用来判断"有没有出声" */
let oscCount = 0;
/** 记录 <audio> 播放尝试 */
let audioPlays: string[] = [];
/** 让 Audio.play() 失败，用于验证退回内置音 */
let audioShouldFail = false;

class FakeOscillator {
  frequency = {
    setValueAtTime: vi.fn(),
    exponentialRampToValueAtTime: vi.fn(),
    linearRampToValueAtTime: vi.fn(),
  };
  type = "sine";
  connect = vi.fn(() => ({ connect: vi.fn() }));
  start = vi.fn();
  stop = vi.fn();
}

class FakeAudioContext {
  currentTime = 0;
  state = "running";
  destination = {};
  resume = vi.fn();
  createOscillator() {
    oscCount++;
    return new FakeOscillator();
  }
  createGain() {
    return {
      gain: {
        setValueAtTime: vi.fn(),
        exponentialRampToValueAtTime: vi.fn(),
      },
      connect: vi.fn(() => ({ connect: vi.fn() })),
    };
  }
}

beforeEach(() => {
  oscCount = 0;
  audioPlays = [];
  audioShouldFail = false;
  invalidateSoundCache();

  vi.stubGlobal("AudioContext", FakeAudioContext);
  vi.stubGlobal("URL", {
    createObjectURL: (_b: unknown) => "blob:fake-url",
    revokeObjectURL: vi.fn(),
  });
  vi.stubGlobal("Blob", class {
    constructor(public parts: unknown[]) {}
  });
  vi.stubGlobal(
    "Audio",
    class {
      volume = 1;
      constructor(public src: string) {}
      play() {
        if (audioShouldFail) return Promise.reject(new Error("decode failed"));
        audioPlays.push(this.src);
        return Promise.resolve();
      }
    }
  );
});

afterEach(() => {
  vi.unstubAllGlobals();
  invalidateSoundCache();
});

// ───────── 音色清单 ─────────

describe("音色清单", () => {
  it("不含任何拟声音效（用户明确不要牛叫那种）", () => {
    // 这条是**需求约束的机器化**：用户 2026-08-27 明确说"我不想要牛叫这种"。
    // 将来有人想加 moo/growl/animal 类音色时，这条会拦住。
    const banned = ["moo", "growl", "bull", "bear", "animal", "牛", "熊"];
    for (const t of TONES) {
      for (const b of banned) {
        expect(t.id.toLowerCase()).not.toContain(b);
        expect(t.label).not.toContain(b);
      }
    }
  });

  it("每个音色都有 label 与 hint（下拉项要给用户判断依据）", () => {
    for (const t of TONES) {
      expect(t.label.length).toBeGreaterThan(0);
      expect(t.hint.length).toBeGreaterThan(0);
    }
  });

  it("id 唯一", () => {
    const ids = TONES.map((t) => t.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("包含静音项，且排在最后", () => {
    expect(TONES.some((t) => t.id === "none")).toBe(true);
    expect(TONES[TONES.length - 1].id).toBe("none");
  });

  it("默认音色 chime 在清单里（后端 Default 与迁移都用它）", () => {
    // 后端 schema.rs 的 Default 与 migrate_v3_to_v4 都硬编码了 "chime"。
    // 若有人把它从清单里删掉，配置里的默认值就会变成"认不出的 id"→ 静音，
    // 表现是"全新安装没有提示音"。
    expect(isToneId("chime")).toBe(true);
  });
});

describe("isToneId", () => {
  it("认得内置 id", () => {
    expect(isToneId("chime")).toBe(true);
    expect(isToneId("wood")).toBe(true);
    expect(isToneId("none")).toBe(true);
  });

  it("不认陌生 id 与自定义值", () => {
    expect(isToneId("moo")).toBe(false);
    expect(isToneId("")).toBe(false);
    expect(isToneId("custom:a.mp3")).toBe(false);
  });
});

describe("customFileName", () => {
  it("取出 custom: 后的文件名", () => {
    expect(customFileName("custom:alarm.mp3")).toBe("alarm.mp3");
  });

  it("内置 id 返回 null", () => {
    expect(customFileName("chime")).toBeNull();
    expect(customFileName("")).toBeNull();
  });

  it("前缀常量与解析一致", () => {
    expect(customFileName(`${CUSTOM_PREFIX}x.wav`)).toBe("x.wav");
  });
});

// ───────── 播放路由（降级行为）─────────

describe("playAlertSound", () => {
  it("内置音色会出声", async () => {
    await playAlertSound("chime", true, 0.8);
    expect(oscCount).toBeGreaterThan(0);
  });

  it("音量 0 时完全不出声", async () => {
    await playAlertSound("chime", true, 0);
    expect(oscCount).toBe(0);
    expect(audioPlays).toEqual([]);
  });

  it("静音音色不出声", async () => {
    await playAlertSound("none", true, 1);
    expect(oscCount).toBe(0);
  });

  it("认不出的音色静音而不抛错", async () => {
    // 后端 sanitize 只校验格式不校验清单，所以陌生 id 会流到前端。
    // 必须静音而不是崩 —— 崩掉会连视觉提醒一起没了。
    await expect(playAlertSound("no-such-tone", true, 1)).resolves.toBeUndefined();
    expect(oscCount).toBe(0);
  });

  it("涨跌都能出声（方向由音高体现，不影响是否出声）", async () => {
    await playAlertSound("marimba", true, 0.5);
    const upCount = oscCount;
    oscCount = 0;
    await playAlertSound("marimba", false, 0.5);
    expect(upCount).toBeGreaterThan(0);
    expect(oscCount).toBe(upCount);
  });

  it("自定义音频经 readCustom 读取并播放", async () => {
    const read = vi.fn(async () => new Uint8Array([1, 2, 3]));
    await playAlertSound("custom:my.mp3", true, 0.6, read);
    expect(read).toHaveBeenCalledWith("my.mp3");
    expect(audioPlays).toEqual(["blob:fake-url"]);
    // 走自定义时不该同时响内置音
    expect(oscCount).toBe(0);
  });

  it("自定义音频只读一次（第二次用缓存）", async () => {
    const read = vi.fn(async () => new Uint8Array([1]));
    await playAlertSound("custom:my.mp3", true, 0.6, read);
    await playAlertSound("custom:my.mp3", true, 0.6, read);
    expect(read).toHaveBeenCalledTimes(1);
    expect(audioPlays).toHaveLength(2);
  });

  it("清缓存后会重新读取", async () => {
    const read = vi.fn(async () => new Uint8Array([1]));
    await playAlertSound("custom:my.mp3", true, 0.6, read);
    invalidateSoundCache("my.mp3");
    await playAlertSound("custom:my.mp3", true, 0.6, read);
    expect(read).toHaveBeenCalledTimes(2);
  });

  it("自定义音频读取失败时**退回内置音**而不是静默", async () => {
    // 这条是关键降级：用户选的文件被删/格式实际不支持时，
    // 必须仍然听得见提醒。完全无声是最难排查的失败方式。
    const read = vi.fn(async () => {
      throw new Error("file not found");
    });
    await playAlertSound("custom:gone.mp3", true, 0.7, read);
    expect(oscCount).toBeGreaterThan(0);
    expect(audioPlays).toEqual([]);
  });

  it("自定义音频播放失败时也退回内置音", async () => {
    audioShouldFail = true;
    const read = vi.fn(async () => new Uint8Array([1]));
    await playAlertSound("custom:bad.mp3", false, 0.7, read);
    expect(oscCount).toBeGreaterThan(0);
  });

  it("自定义音色但没传 readCustom 时不崩", async () => {
    await expect(
      playAlertSound("custom:x.mp3", true, 0.5)
    ).resolves.toBeUndefined();
  });

  it("音量超范围被钳制而不是放大爆音", async () => {
    // 配置文件用户可手改。后端 sanitize 会钳，但前端也要自保 ——
    // 传 5.0 进来不该让 Audio.volume 抛 IndexSizeError
    await expect(playAlertSound("chime", true, 5)).resolves.toBeUndefined();
    expect(oscCount).toBeGreaterThan(0);
    const read = vi.fn(async () => new Uint8Array([1]));
    await expect(
      playAlertSound("custom:a.mp3", true, 99, read)
    ).resolves.toBeUndefined();
  });

  it("AudioContext 不可用时静默失败，不影响调用方", async () => {
    vi.stubGlobal(
      "AudioContext",
      class {
        constructor() {
          throw new Error("no audio device");
        }
      }
    );
    await expect(playAlertSound("chime", true, 1)).resolves.toBeUndefined();
  });
});
