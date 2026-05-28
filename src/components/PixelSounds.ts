/**
 * Pixel Sound Effects — 8-bit style sounds for different events.
 * Uses Web Audio API to generate retro game sounds.
 */

let _ctx: AudioContext | null = null;

function ctx(): AudioContext {
  if (!_ctx || _ctx.state === "closed") _ctx = new AudioContext();
  if (_ctx.state === "suspended") _ctx.resume();
  return _ctx;
}

function playTone(freq: number, duration: number, type: OscillatorType = "square", vol = 0.12) {
  try {
    const c = ctx();
    const osc = c.createOscillator();
    const gain = c.createGain();
    osc.connect(gain);
    gain.connect(c.destination);
    osc.type = type;
    osc.frequency.setValueAtTime(freq, c.currentTime);
    gain.gain.setValueAtTime(vol, c.currentTime);
    gain.gain.exponentialRampToValueAtTime(0.001, c.currentTime + duration);
    osc.start(c.currentTime);
    osc.stop(c.currentTime + duration);
  } catch {}
}

function playNotes(notes: [number, number, OscillatorType?][], vol = 0.1) {
  try {
    const c = ctx();
    let t = c.currentTime;
    for (const [freq, dur, type] of notes) {
      const osc = c.createOscillator();
      const gain = c.createGain();
      osc.connect(gain);
      gain.connect(c.destination);
      osc.type = type || "square";
      osc.frequency.setValueAtTime(freq, t);
      gain.gain.setValueAtTime(vol, t);
      gain.gain.exponentialRampToValueAtTime(0.001, t + dur);
      osc.start(t);
      osc.stop(t + dur);
      t += dur;
    }
  } catch {}
}

/** Permission request — ascending 3-note chime */
export function playPermissionSound() {
  playNotes([
    [523, 0.08],  // C5
    [659, 0.08],  // E5
    [784, 0.12],  // G5
  ], 0.1);
}

/** Tool start — quick beep */
export function playToolStartSound() {
  playTone(880, 0.06, "square", 0.08);
}

/** Completion — happy 4-note jingle */
export function playCompleteSound() {
  playNotes([
    [523, 0.06],  // C5
    [659, 0.06],  // E5
    [784, 0.06],  // G5
    [1047, 0.1],  // C6
  ], 0.08);
}

/** Error / deny — descending buzz */
export function playErrorSound() {
  playNotes([
    [440, 0.08, "sawtooth"],
    [349, 0.08, "sawtooth"],
    [294, 0.12, "sawtooth"],
  ], 0.06);
}

/** Notification — soft ping */
export function playNotifySound() {
  playTone(1047, 0.1, "sine", 0.06);
}

/** Unlock audio context on first user interaction */
export function unlockAudio() {
  ctx();
}
