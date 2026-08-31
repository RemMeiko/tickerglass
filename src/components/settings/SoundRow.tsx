import { useRef, useState } from "react";
import {
  CUSTOM_PREFIX,
  TONES,
  customFileName,
  invalidateSoundCache,
  playAlertSound,
} from "../../lib/sounds";
import {
  deleteAlertSound,
  importAlertSound,
  readAlertSound,
} from "../../lib/ipc";

interface Props {
  label: string;
  enabled: boolean;
  /** 当前音色：内置 id 或 `custom:文件名` */
  tone: string;
  volume: number;
  /** 已导入的自定义音频文件名 */
  customs: string[];
  /** true=上涨（试听时用上行音高） */
  up: boolean;
  onToggle: (v: boolean) => void;
  onTone: (v: string) => void;
  /** 导入/删除后通知父组件刷新 customs 列表 */
  onImported: () => void;
}

/** 后端也校验，但前端先拦一道能给出更快的反馈 */
const ACCEPT = ".mp3,.wav,.m4a,.aac,.ogg";
const MAX_BYTES = 5 * 1024 * 1024;

/**
 * 一组提示音设置：开关 + 音色下拉 + 试听 + 导入自定义音频。
 *
 * 上涨与下跌各用一个实例，逻辑完全相同只差 `up` —— 抽成组件而不是复制两遍，
 * 否则改一处忘另一处（这个面板原来就是两段几乎一样的代码）。
 */
export default function SoundRow({
  label,
  enabled,
  tone,
  volume,
  customs,
  up,
  onToggle,
  onTone,
  onImported,
}: Props) {
  const fileRef = useRef<HTMLInputElement | null>(null);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);

  function notify(m: string) {
    setMsg(m);
    window.setTimeout(() => setMsg((c) => (c === m ? null : c)), 3000);
  }

  async function preview() {
    // 试听走与实际提醒**同一个函数**，避免"试听正常但提醒时不一样"
    await playAlertSound(tone, up, volume, readAlertSound);
  }

  async function pick(e: React.ChangeEvent<HTMLInputElement>) {
    const f = e.target.files?.[0];
    // 清掉 input 的值：不清的话选同一个文件第二次不会触发 change
    e.target.value = "";
    if (!f) return;
    if (f.size > MAX_BYTES) {
      notify(`文件过大（${(f.size / 1024 / 1024).toFixed(1)} MB），上限 5 MB`);
      return;
    }
    setBusy(true);
    try {
      const buf = new Uint8Array(await f.arrayBuffer());
      const saved = await importAlertSound(f.name, buf);
      // 覆盖导入同名文件时旧 Blob 还在缓存里，必须失效掉
      invalidateSoundCache(saved);
      onTone(`${CUSTOM_PREFIX}${saved}`);
      onImported();
      notify(`已导入 ${saved}`);
    } catch (err) {
      notify(String(err));
    } finally {
      setBusy(false);
    }
  }

  async function removeCustom() {
    const name = customFileName(tone);
    if (!name) return;
    setBusy(true);
    try {
      await deleteAlertSound(name);
      invalidateSoundCache(name);
      // 删掉后退回默认音色，否则配置里留着一个指向已删文件的值
      onTone("chime");
      onImported();
      notify(`已删除 ${name}`);
    } catch (err) {
      notify(String(err));
    } finally {
      setBusy(false);
    }
  }

  const currentCustom = customFileName(tone);

  return (
    <div className="snd-row">
      <label className="row-switch snd-switch">
        <input
          type="checkbox"
          checked={enabled}
          onChange={(e) => onToggle(e.target.checked)}
        />
        <span>{label}</span>
      </label>

      <div className="snd-controls">
        <select
          className="snd-select"
          value={tone}
          disabled={!enabled}
          onChange={(e) => onTone(e.target.value)}
        >
          <optgroup label="内置音色">
            {TONES.map((t) => (
              <option key={t.id} value={t.id}>
                {t.label} — {t.hint}
              </option>
            ))}
          </optgroup>
          {customs.length > 0 && (
            <optgroup label="我的音频">
              {customs.map((c) => (
                <option key={c} value={`${CUSTOM_PREFIX}${c}`}>
                  {c}
                </option>
              ))}
            </optgroup>
          )}
        </select>

        <button
          className="snd-btn"
          disabled={!enabled || busy}
          onClick={() => void preview()}
          title="按当前音色与音量试听"
        >
          试听
        </button>

        <button
          className="snd-btn"
          disabled={!enabled || busy}
          onClick={() => fileRef.current?.click()}
          title="选一个本地音频文件（mp3/wav/m4a/aac/ogg，≤5MB）"
        >
          选文件…
        </button>

        {currentCustom && (
          <button
            className="snd-btn snd-btn-danger"
            disabled={busy}
            onClick={() => void removeCustom()}
            title={`删除 ${currentCustom}`}
          >
            删除
          </button>
        )}

        {/* 隐藏的原生文件选择器。用它而不是 tauri-plugin-dialog：
            webview 里 <input type="file"> 就能选文件并读出字节，零新依赖。 */}
        <input
          ref={fileRef}
          type="file"
          accept={ACCEPT}
          style={{ display: "none" }}
          onChange={(e) => void pick(e)}
        />
      </div>

      {msg && <div className="snd-msg">{msg}</div>}
    </div>
  );
}
