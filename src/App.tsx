import { useState, useEffect, useCallback, useMemo, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { PixelPet, getPetState } from "./components/PixelPet";
import type { PetState } from "./components/PixelPet";
import { playPermissionSound, unlockAudio } from "./components/PixelSounds";
import "./styles/island.css";

interface PermissionRequest { id: string; title?: string; summary?: string; tool_name?: string; tool_use_id?: string; affected_path?: string; primary_action_title?: string; secondary_action_title?: string; permission_suggestions?: unknown; }
interface QuestionOption { label: string; description?: string; }
interface QuestionPrompt { title?: string; question: string; options: QuestionOption[]; source: string; }
interface ToolHistoryEntry { tool: string; description?: string; started_at: number; finished_at?: number; success: boolean; }
interface AgentSession {
  id: string; title: string; agent: string; phase: string; tool: string; current_tool?: string;
  summary?: string; permission_request?: PermissionRequest; question_prompt?: QuestionPrompt;
  origin: string | null; updated_at: number; first_seen_at: number; terminal_app: string | null;
  tool_history?: ToolHistoryEntry[]; last_user_prompt?: string; last_assistant_message?: string;
}
interface SessionStateSnapshot { sessions: AgentSession[]; running_count: number; waiting_count: number; done_count: number; }
type GroupBy = "none" | "state" | "agent";
type IslandWidth = "compact" | "standard" | "wide";
type SessionDisplay = "compact" | "detailed";
type NotchState = "compact" | "overview" | "approval" | "ask" | "settings" | "approved";
type Lang = "zh" | "en";

function phaseColor(p: string) { return { Running: "#3b82f6", WaitingForApproval: "#f97316", WaitingForAnswer: "#06b6d4", Completed: "#22c55e" }[p] || "#737373"; }
function fmtTime(ts: number) { const m = Math.floor((Date.now() - ts) / 60000); if (m < 1) return "< 1m"; if (m < 60) return m + "m"; return Math.floor(m / 60) + "h" + (m % 60) + "m"; }
function fmtAgent(a: string) { return ({ ClaudeCode: "Claude", Codex: "Codex", GeminiCLI: "Gemini", OpenCode: "OpenCode", Cursor: "Cursor", KimiCLI: "Kimi", QwenCode: "Qwen" } as Record<string,string>)[a] || a; }
function getLang(): Lang { const s = localStorage.getItem("open-island-lang"); if (s === "en" || s === "zh") return s; return navigator.language.startsWith("zh") ? "zh" : "en"; }
const I18N: Record<Lang, Record<string, string>> = {
  zh: { title: "会话", empty: "暂无活跃会话", hint: "在终端中启动 AI 编程助手", perm: "请求工具权限", allow: "允许", deny: "拒绝", always: "始终允许", answer: "需要回答", settings: "设置", autostart: "开机自启", "auto.approve": "自动批准权限", language: "语言", general: "通用", display: "显示", about: "关于", "island.size": "岛屿大小", "session.view": "会话视图", "view.compact": "精简", "view.detailed": "详细", group: "分组方式", "g.none": "不分组", "g.state": "按状态", "g.agent": "按 Agent", install: "安装", running: "运行", waiting: "等待", done: "完成" },
  en: { title: "Sessions", empty: "No active sessions", hint: "Start an AI coding assistant in terminal", perm: "Tool Permission", allow: "Allow", deny: "Deny", always: "Always", answer: "Answer Required", settings: "Settings", autostart: "Auto-start", "auto.approve": "Auto-approve permissions", language: "Language", general: "General", display: "Display", about: "About", "island.size": "Island size", "session.view": "Session view", "view.compact": "Compact", "view.detailed": "Detailed", group: "Group By", "g.none": "None", "g.state": "State", "g.agent": "Agent", install: "Install", running: "Running", waiting: "Waiting", done: "Done" },
};
function T(key: string, lang?: Lang) { return I18N[lang || getLang()][key] || key; }
const SAFE_TOOLS = new Set(["Read", "Grep", "Glob"]);
const WRITE_TOOLS = new Set(["Write", "Edit", "MultiEdit"]);
const DESTRUCTIVE = /\b(rm\s+-rf|del\s+\/[fsq]|git\s+reset\s+--hard|format|mkfs)\b/i;
function assessRisk(tool: string, summary?: string) {
  if (SAFE_TOOLS.has(tool)) return { level: "safe" as const, signal: "READ" };
  if (tool === "Bash") return { level: (summary && DESTRUCTIVE.test(summary) ? "danger" : "review") as "danger" | "review", signal: "SHELL" };
  if (WRITE_TOOLS.has(tool)) return { level: "review" as const, signal: "WRITE" };
  return { level: "review" as const, signal: "TOOL" };
}

export default function App() {
  const [state, setState] = useState<NotchState>("compact");
  const [sessions, setSessions] = useState<AgentSession[]>([]);
  const [counts, setCounts] = useState({ running: 0, waiting: 0, done: 0 });
  const [hooks, setHooks] = useState<Record<string, boolean>>({});
  const [autostart, setAutostart] = useState(false);
  const [autoApprove, setAutoApprove] = useState(false);
  const [sound, setSound] = useState(true);
  const [groupBy, setGroupBy] = useState<GroupBy>("state");
  const [islandWidth, setIslandWidth] = useState<IslandWidth>("standard");
  const [sessionDisplay, setSessionDisplay] = useState<SessionDisplay>("detailed");
  const [lang, setLang] = useState<Lang>(getLang());
  const [approvedLabel, setApprovedLabel] = useState("Approved");
  const prevWaiting = useRef(0);
  const collapseTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const petState = getPetState(counts.running, counts.waiting);
  const activeSession = useMemo(() => sessions.find(s => s.phase === "WaitingForApproval" || s.phase === "WaitingForAnswer") || sessions.find(s => s.phase === "Running") || sessions[0], [sessions]);
  const waitingApproval = useMemo(() => sessions.find(s => s.phase === "WaitingForApproval" && s.permission_request), [sessions]);
  const waitingAnswer = useMemo(() => sessions.find(s => s.phase === "WaitingForAnswer" && s.question_prompt), [sessions]);

  const transitionTo = useCallback((next: NotchState) => {
    if (collapseTimer.current) { clearTimeout(collapseTimer.current); collapseTimer.current = null; }
    setState(next);
    invoke("resize_window", { state: next }).catch(() => {});
  }, []);

  useEffect(() => {
    if (waitingApproval && state !== "approval" && state !== "approved") {
      transitionTo("approval");
    } else if (waitingAnswer && state !== "ask" && state !== "approved") {
      transitionTo("ask");
    }
  }, [waitingApproval, waitingAnswer, transitionTo, state]);

  useEffect(() => {
    const unlisten = listen<SessionStateSnapshot>("state-changed", (e) => {
      const prev = prevWaiting.current;
      setSessions(e.payload.sessions);
      setCounts({ running: e.payload.running_count, waiting: e.payload.waiting_count, done: e.payload.done_count });
      if (sound && e.payload.waiting_count > prev) playPermissionSound();
      prevWaiting.current = e.payload.waiting_count;
    });
    return () => { unlisten.then(fn => fn()); };
  }, [sound]);

  useEffect(() => {
    invoke<SessionStateSnapshot>("get_state").then(s => { setSessions(s.sessions); setCounts({ running: s.running_count, waiting: s.waiting_count, done: s.done_count }); }).catch(() => {});
    invoke<Record<string, boolean>>("get_hook_status").then(setHooks).catch(() => {});
    invoke<boolean>("get_autostart_status").then(setAutostart).catch(() => {});
    invoke<boolean>("get_auto_approve").then(setAutoApprove).catch(() => {});
    const sl = localStorage.getItem("open-island-sound"); if (sl !== null) setSound(sl === "true");
    const sw = localStorage.getItem("open-island-width") as IslandWidth | null; if (sw) setIslandWidth(sw);
    const sd = localStorage.getItem("open-island-display") as SessionDisplay | null; if (sd) setSessionDisplay(sd);
    const sL = localStorage.getItem("open-island-lang"); if (sL === "en" || sL === "zh") setLang(sL);
    const u = () => { unlockAudio(); document.removeEventListener("click", u); document.removeEventListener("keydown", u); };
    document.addEventListener("click", u, { once: true }); document.addEventListener("keydown", u, { once: true });
  }, []);

  useEffect(() => {
    if (state === "compact") return;
    const unlisten = listen("tauri://blur", () => {
      if (collapseTimer.current) clearTimeout(collapseTimer.current);
      collapseTimer.current = setTimeout(() => { collapseTimer.current = null; setState("compact"); invoke("resize_window", { state: "compact" }); }, 150);
    });
    return () => { unlisten.then(fn => fn()); };
  }, [state]);


  // Auto-return to overview 2 seconds after reaching "approved" state
  useEffect(() => {
    if (state !== "approved") return;
    const timer = setTimeout(() => transitionTo("overview"), 2000);
    return () => clearTimeout(timer);
  }, [state, transitionTo]);

  const handleNotchClick = useCallback(() => {
    if (state === "compact") {
      transitionTo("overview");
    }
  }, [state, transitionTo]);

  const compactText = activeSession?.title || activeSession?.summary?.slice(0, 20) || T("title", lang);

  return (
    <div className="island-container">
      {state !== "compact" && <div className="backdrop" onClick={() => transitionTo("compact")} />}
      <div className="island">
        <div className="island__notch" data-state={state} onClick={handleNotchClick}>
          {/* Compact */}
          <div className={"v6-layer" + (state === "compact" ? " v6-layer-active" : "")} data-layer="compact">
            <div className="compact-pet"><PixelPet state={petState} size={26} /></div>
            <span className="compact-text">{compactText}</span>
            {sessions.length > 0 && <span className="compact-count">{sessions.length}</span>}
          </div>

          {/* Overview */}
          <div className={"v6-layer" + (state === "overview" ? " v6-layer-active" : "")} data-layer="overview" onClick={e => { if (e.target === e.currentTarget) transitionTo("compact"); e.stopPropagation(); }}>
            {activeSession && <HeroSession session={activeSession} onJump={() => invoke("jump_to_terminal", { sessionId: activeSession.id }).catch(() => {})} />}
            {sessions.filter(s => s.id !== activeSession?.id).slice(0, 3).map(s => <MiniSession key={s.id} session={s} onJump={() => invoke("jump_to_terminal", { sessionId: s.id }).catch(() => {})} />)}
            {sessions.length === 0 && <div className="empty-layer"><div className="empty-layer__text">{T("empty", lang)}</div><div className="empty-layer__hint">{T("hint", lang)}</div></div>}
            <div style={{ display: "flex", justifyContent: "flex-end", padding: "2px 0 0" }}><button className="icon-btn" onClick={e => { e.stopPropagation(); transitionTo("settings"); }}><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="M12.22 2h-.44a2 2 0 00-2 2v.18a2 2 0 01-1 1.73l-.43.25a2 2 0 01-2 0l-.15-.08a2 2 0 00-2.73.73l-.22.38a2 2 0 00.73 2.73l.15.1a2 2 0 011 1.72v.51a2 2 0 01-1 1.74l-.15.09a2 2 0 00-.73 2.73l.22.38a2 2 0 002.73.73l.15-.08a2 2 0 012 0l.43.25a2 2 0 011 1.73V20a2 2 0 002 2h.44a2 2 0 002-2v-.18a2 2 0 011-1.73l.43-.25a2 2 0 012 0l.15.08a2 2 0 002.73-.73l.22-.39a2 2 0 00-.73-2.73l-.15-.08a2 2 0 01-1-1.74v-.5a2 2 0 011-1.74l.15-.09a2 2 0 00.73-2.73l-.22-.38a2 2 0 00-2.73-.73l-.15.08a2 2 0 01-2 0l-.43-.25a2 2 0 01-1-1.73V4a2 2 0 00-2-2z"/><circle cx="12" cy="12" r="3"/></svg></button></div>
          </div>

          {/* Approval */}
          <div className={"v6-layer" + (state === "approval" ? " v6-layer-active" : "")} data-layer="approval" onClick={e => { if (e.target === e.currentTarget) transitionTo("compact"); e.stopPropagation(); }}>
            {waitingApproval && <ApprovalContent session={waitingApproval}
              onAllow={() => { invoke("approve_permission", { sessionId: waitingApproval.id, always: false }).catch(() => {}); setApprovedLabel("Approved"); transitionTo("approved"); }}
              onDeny={() => { invoke("deny_permission", { sessionId: waitingApproval.id }).catch(() => {}); transitionTo("compact"); }}
              onAlways={() => { invoke("approve_permission", { sessionId: waitingApproval.id, always: true }).catch(() => {}); setApprovedLabel("Always Allowed"); transitionTo("approved"); }}
            />}
          </div>

          {/* Ask */}
          <div className={"v6-layer" + (state === "ask" ? " v6-layer-active" : "")} data-layer="ask" onClick={e => { if (e.target === e.currentTarget) transitionTo("compact"); e.stopPropagation(); }}>
            {waitingAnswer && <AskContent session={waitingAnswer} lang={lang} onAnswer={ans => { invoke("answer_question", { sessionId: waitingAnswer.id, answer: ans }).catch(() => {}); setApprovedLabel(ans); transitionTo("approved"); }} />}
          </div>

          {/* Approved */}
          <div className={"v6-layer" + (state === "approved" ? " v6-layer-active" : "")} data-layer="approved" onClick={() => transitionTo("compact")}>
            <span className="v6-approved-check">{"\u2713"}</span>
            <span className="v6-approved-text">{approvedLabel}</span>
          </div>

          {/* Settings */}
          <div className={"v6-layer" + (state === "settings" ? " v6-layer-active" : "")} data-layer="settings" onClick={e => { if (e.target === e.currentTarget) transitionTo("compact"); e.stopPropagation(); }}>
            <SettingsContent autostart={autostart} autoApprove={autoApprove} hooks={hooks} groupBy={groupBy} islandWidth={islandWidth} sessionDisplay={sessionDisplay} lang={lang}
              onInstall={a => invoke("install_hooks", { agent: a }).then(() => invoke<Record<string, boolean>>("get_hook_status").then(setHooks)).catch(() => {})}
              onAutostart={() => invoke("toggle_autostart").then(() => invoke<boolean>("get_autostart_status").then(setAutostart)).catch(() => {})}
              onAutoApprove={() => invoke("toggle_auto_approve").then(() => invoke<boolean>("get_auto_approve").then(setAutoApprove)).catch(() => {})}
              onGroup={setGroupBy} onWidthChange={w => { setIslandWidth(w); localStorage.setItem("open-island-width", w); }}
              onDisplayChange={d => { setSessionDisplay(d); localStorage.setItem("open-island-display", d); }}
              onLangChange={l => { setLang(l); localStorage.setItem("open-island-lang", l); }}
              onBack={() => transitionTo("overview")} />
          </div>
        </div>
      </div>
    </div>
  );
}

function HeroSession({ session: s, onJump }: { session: AgentSession; onJump: () => void }) {
  const color = phaseColor(s.phase);
  const ps: PetState = s.phase === "Running" ? "working" : s.phase === "Completed" ? "idle" : "waiting";
  return (
    <div className="v6-sess v6-sess-hero" onClick={onJump}>
      <div style={{ flexShrink: 0, marginTop: 4 }}><PixelPet state={ps} size={16} /></div>
      <div className="v6-si">
        <div className="v6-sess-r1"><span className="v6-sn">{s.title || s.summary?.slice(0, 20) || s.agent}</span><span className="v6-st">{fmtAgent(s.agent)}</span>{s.terminal_app && <span className="v6-st">{s.terminal_app}</span>}<span className="v6-sess-dur">{fmtTime(s.updated_at)}</span></div>
        {s.last_user_prompt && <div className="v6-sess-you">{"You: " + s.last_user_prompt.slice(0, 60)}</div>}
        <div className="v6-ss" style={{ color }}>{s.phase === "Completed" ? "Done \u2014 click to jump" : s.current_tool || s.tool}</div>
      </div>
    </div>
  );
}

function MiniSession({ session: s, onJump }: { session: AgentSession; onJump: () => void }) {
  const color = phaseColor(s.phase);
  return (
    <div className="v6-sess v6-sess-mini" onClick={onJump}>
      <div className="v6-sd" style={{ background: color }} />
      <span className="v6-sn">{s.title || s.summary?.slice(0, 20) || s.agent}</span>
      <div className="v6-sess-tags"><span className="v6-st">{fmtAgent(s.agent)}</span>{s.terminal_app && <span className="v6-st">{s.terminal_app}</span>}<span className="v6-sess-dur">{fmtTime(s.updated_at)}</span></div>
    </div>
  );
}

function ApprovalContent({ session: s, onAllow, onDeny, onAlways }: { session: AgentSession; onAllow: () => void; onDeny: () => void; onAlways: () => void }) {
  const risk = s.permission_request ? assessRisk(s.permission_request.tool_name || s.tool, s.permission_request.summary) : null;
  return (
    <>
      <div className="v6-appr-head"><div className="v6-appr-dot" /><span>Permission Request</span></div>
      <div className="v6-appr-tool"><span className="v6-appr-icon">{"\u26A1"}</span><span className="v6-appr-name">{s.permission_request?.tool_name || s.tool}</span><span className="v6-appr-input">{s.permission_request?.title || s.summary}</span></div>
      {s.permission_request?.summary && <div className="v6-appr-ctx"><div className="v6-diff"><div className="v6-diff-line v6-diff-ctx"><span className="v6-diff-ln" />{s.permission_request.summary.slice(0, 200)}</div></div>{risk && <span className="v6-appr-diff">{risk.signal}</span>}</div>}
      <div className="v6-appr-btns">
        <button className="v6-btn-deny" onClick={onDeny}>Deny</button>
        <button className="v6-btn-allow" onClick={onAllow}>Allow <kbd>{"\u2318"}Y</kbd></button>
        <button className="v6-btn-deny" onClick={onAlways} style={{ fontSize: "10px" }}>Always</button>
      </div>
    </>
  );
}

function AskContent({ session: s, lang, onAnswer }: { session: AgentSession; lang: Lang; onAnswer: (a: string) => void }) {
  const qp = s.question_prompt;
  const isDesktop = qp?.source === "desktop";
  const hasOptions = qp && qp.options.length > 0;
  const questionText = qp?.question || s.last_user_prompt || s.summary || T("answer", lang);

  // Claude Desktop: show "go back to Claude Desktop" reminder
  if (isDesktop || !hasOptions) {
    return (
      <>
        <div className="v6-ask-head"><svg width="12" height="12" viewBox="0 0 24 24" fill="var(--vi-question)" opacity=".9"><path d="M21 15a2 2 0 01-2 2H7l-4 4V5a2 2 0 012-2h14a2 2 0 012 2z" /></svg><span>{fmtAgent(s.agent)} {lang === "zh" ? "\u9700\u8981\u56de\u7b54" : "asks"}</span></div>
        <div className="v6-ask-q" style={{ fontSize: 12, lineHeight: 1.5 }}>{questionText.length > 120 ? questionText.slice(0, 120) + "..." : questionText}</div>
        <div className="v6-ask-desktop-hint">
          <div className="v6-ask-desktop-icon">{"\u2197"}</div>
          <div className="v6-ask-desktop-text">{lang === "zh" ? "\u8bf7\u8fd4\u56de Claude Desktop \u8fdb\u884c\u9009\u62e9" : "Go back to Claude Desktop to answer"}</div>
          <div className="v6-ask-desktop-sub">{lang === "zh" ? "\u9009\u62e9\u540e\u5c06\u81ea\u52a8\u6062\u590d" : "Will resume automatically after you answer"}</div>
        </div>
      </>
    );
  }

  // Claude Code: show answer options
  return (
    <>
      <div className="v6-ask-head"><svg width="12" height="12" viewBox="0 0 24 24" fill="var(--vi-question)" opacity=".9"><path d="M21 15a2 2 0 01-2 2H7l-4 4V5a2 2 0 012-2h14a2 2 0 012 2z" /></svg><span>{fmtAgent(s.agent)} asks</span></div>
      <div className="v6-ask-q">{questionText}</div>
      <div className="v6-ask-opts">
        {qp.options.map((opt, i) => (
          <button key={i} className="v6-ask-opt" onClick={() => onAnswer(opt.label)}>
            <span className="v6-ask-num">{"\u2318"}{i + 1}</span>
            <span>{opt.label}</span>
          </button>
        ))}
        {qp.options.length === 0 && <>
          <button className="v6-ask-opt" onClick={() => onAnswer("yes")}><span className="v6-ask-num">{"\u2318"}1</span><span>Yes</span></button>
          <button className="v6-ask-opt" onClick={() => onAnswer("no")}><span className="v6-ask-num">{"\u2318"}2</span><span>No</span></button>
        </>}
      </div>
    </>
  );
}

function SettingsContent({ autostart, autoApprove, hooks, groupBy, islandWidth, sessionDisplay, lang, onInstall, onAutostart, onAutoApprove, onGroup, onWidthChange, onDisplayChange, onLangChange, onBack }: {
  autostart: boolean; autoApprove: boolean; hooks: Record<string, boolean>; groupBy: GroupBy; islandWidth: IslandWidth; sessionDisplay: SessionDisplay; lang: Lang;
  onInstall: (a: string) => void; onAutostart: () => void; onAutoApprove: () => void; onGroup: (g: GroupBy) => void; onWidthChange: (w: IslandWidth) => void; onDisplayChange: (d: SessionDisplay) => void; onLangChange: (l: Lang) => void; onBack: () => void;
}) {
  return (
    <>
      <div className="settings-topbar"><button className="settings-back" onClick={onBack}>{"\u2190"}</button><h3 className="settings-title">{T("settings", lang)}</h3><span className="settings-version">v0.1.0</span></div>
      <div className="settings-section"><h4>{T("language", lang)}</h4><div className="settings-radio-group">{(["zh", "en"] as Lang[]).map(l => <button key={l} className={"settings-radio" + (lang === l ? " settings-radio--active" : "")} onClick={() => onLangChange(l)}>{l === "zh" ? "\u4E2D\u6587" : "EN"}</button>)}</div></div>
      <div className="settings-section"><h4>{T("general", lang)}</h4><label className="settings-toggle"><input type="checkbox" checked={autostart} onChange={onAutostart} /><span>{T("autostart", lang)}</span></label><label className="settings-toggle"><input type="checkbox" checked={autoApprove} onChange={onAutoApprove} /><span>{T("auto.approve", lang)}</span></label></div>
      <div className="settings-section"><h4>{T("display", lang)}</h4>
        <div className="settings-row"><span className="settings-row-label">{T("island.size", lang)}</span><div className="settings-radio-group">{(["compact", "standard", "wide"] as IslandWidth[]).map(w => <button key={w} className={"settings-radio" + (islandWidth === w ? " settings-radio--active" : "")} onClick={() => onWidthChange(w)}>{w === "compact" ? "S" : w === "standard" ? "M" : "L"}</button>)}</div></div>
        <div className="settings-row"><span className="settings-row-label">{T("session.view", lang)}</span><div className="settings-radio-group">{(["compact", "detailed"] as SessionDisplay[]).map(d => <button key={d} className={"settings-radio" + (sessionDisplay === d ? " settings-radio--active" : "")} onClick={() => onDisplayChange(d)}>{T("view." + d, lang)}</button>)}</div></div>
      </div>
      <div className="settings-section"><h4>{T("group", lang)}</h4><div className="settings-radio-group">{(["none", "state", "agent"] as GroupBy[]).map(g => <button key={g} className={"settings-radio" + (groupBy === g ? " settings-radio--active" : "")} onClick={() => onGroup(g)}>{T("g." + g, lang)}</button>)}</div></div>
      <div className="settings-section"><h4>CLI Hooks</h4>{["claude", "codex", "gemini", "cursor"].map(a => <div key={a} className="settings-hook-row"><span className="settings-hook-name">{a}</span>{hooks[a] ? <span className="settings-hook-status--installed">{"\u2713"}</span> : <button className="settings-hook-btn" onClick={() => onInstall(a)}>{T("install", lang)}</button>}</div>)}</div>
      <div className="settings-section settings-section--info"><h4>{T("about", lang)}</h4><div className="settings-info"><span>Open Island Windows</span><span className="settings-info-detail">AI Agent Dynamic Island</span></div></div>
    </>
  );
}
