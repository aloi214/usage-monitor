import { createLogSourceCatalog, createLogSourceHandlers, renderLogSourceSettings, projectScanSources, type ScanSourceStatus, type ScanSourceSettings } from "./log-source-settings";
import { createProviderModeCatalog, renderProviderModeCatalogStatus, createProviderModeHandlers, renderProviderMode, dropModeSnapshots, localModeSelection, type ProviderModeCatalogEntry } from "./provider-modes";
import { acceptPriceEpoch, mergeRepricedLocal, estimatedCostText } from "./manual-pricing";
import { scanSpendVisible, acceptSpendBatch } from "./scan-consent";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getVersion } from "@tauri-apps/api/app";
import { applyWidgetState, initWidget } from "./widget";
import {
  applyStaticI18n,
  displayLinkLabel,
  displayMetricDetail,
  displayMetricLabel,
  displayLocalStatValue,
  displayLocalSourceStatus,
  displayScanError,
  localeTag,
  normalizeLocalePref,
  resolveLocale,
  setActiveLocale,
  setSystemLocale,
  t,
  type Locale,
  type LocalePref,
} from "./i18n";

// Injected by vite.config.ts at build time, e.g. "0707.1432".
declare const __BUILD_STAMP__: string;

// Official provider marks from the MIT-licensed macOS OpenUsage, rendered
// inline so CSS can recolor them like template icons. CommandCode uses
// a neutral code symbol for its experimental integration.
import antigravityIcon from "./assets/providers/antigravity.svg?raw";
import claudeIcon from "./assets/providers/claude.svg?raw";
import commandcodeIcon from "./assets/providers/commandcode.svg?raw";
import codexIcon from "./assets/providers/codex.svg?raw";
import copilotIcon from "./assets/providers/copilot.svg?raw";
import cursorIcon from "./assets/providers/cursor.svg?raw";
import devinIcon from "./assets/providers/devin.svg?raw";
import grokIcon from "./assets/providers/grok.svg?raw";
import hermesIcon from "./assets/providers/hermes.svg?raw";
import kimiIcon from "./assets/providers/kimi.svg?raw";
import minimaxIcon from "./assets/providers/minimax.svg?raw";
import stepfunIcon from "./assets/providers/stepfun.svg?raw";
import opencodeIcon from "./assets/providers/opencode.svg?raw";
import openrouterIcon from "./assets/providers/openrouter.svg?raw";
// Inlined as data URIs (not URLs) so the share-card SVG snapshot can
// embed them — rasterized SVG images can't load external resources.
// The bare ring suits the sidebar; the footer uses the full rounded
// app icon, which stays legible at tiny sizes.
import paneLogo from "./assets/pane-logo.png?inline";
import paneIcon from "./assets/pane-icon.png?inline";
import zaiIcon from "./assets/providers/zai.svg?raw";
// The repo's changelog ships inside the bundle, so the "What's new" dialog
// and the Settings changelog viewer read the exact file releases maintain.
import changelogRaw from "../CHANGELOG.md?raw";

const PROVIDER_ICONS: Record<string, string> = {
  antigravity: antigravityIcon,
  claude: claudeIcon,
  commandcode: commandcodeIcon,
  codex: codexIcon,
  copilot: copilotIcon,
  cursor: cursorIcon,
  devin: devinIcon,
  grok: grokIcon,
  hermes: hermesIcon,
  kimi: kimiIcon,
  minimax: minimaxIcon,
  opencode: opencodeIcon,
  openrouter: openrouterIcon,
  stepfun: stepfunIcon,
  zai: zaiIcon,
};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface Metric {
  label: string;
  kind: string; // "progress" | "text" | "action" | "resets"
  used_percent: number | null;
  detail: string | null;
  value: string | null;
  resets_at: number | null;
  period_ms: number | null;
  /** Set when resets_at is an expiry — the value is lost, not renewed. */
  expires?: boolean;
}

/// One banked reset credit inside a "resets" row's detail JSON. `id` is
/// present only when the credit can be redeemed (Codex); Grok's are
/// read-only.
interface ResetCredit {
  id?: string;
  expires_at: number | null;
}

interface Snapshot {
  id: string;
  name: string;
  plan: string | null;
  status: string;
  error: string | null;
  metrics: Metric[];
  stale: boolean;
  warning: string | null;
  fetched_at?: number | null;
  attempt_failed?: boolean;
  dashboard_url?: string | null;
  wallet_history?: { source_account_id?: string; fetched_at?: number | null; attempted_at?: number | null; warning?: string | null } | null;
}

interface ModelSpend {
  model: string;
  cost: number;
  tokens: number;
  unpriced: boolean;
}

interface SpendWindow {
  cost: number;
  tokens: number;
  models: ModelSpend[];
}

interface SpendResult { revision: number; rows: ProviderSpend[]; preserveCursor?: boolean; }
interface PricingStatus { last_success_ms: number; catalog_stamp: string; }
interface PricingUpdate { pricing: PricingStatus; spend: SpendResult; }

interface ProviderSpend {
  sources: string[];
  scan_revision: number | null;
  local_stats: { label: string; value: string }[];
  id: string;
  name: string;
  today: SpendWindow;
  yesterday: SpendWindow;
  last30: SpendWindow;
  trend: number[];
  unpriced: number;
  unpriced_models: string[];
  month_cost: number;
}

/// How to get each provider signed in again, for the ⚠ Outdated tooltip.
const RELOGIN_KEYS: Record<string, string> = {
  claude: "stale.relogin.claude",
  commandcode: "stale.relogin.commandcode",
  codex: "stale.relogin.codex",
  grok: "stale.relogin.grok",
  copilot: "stale.relogin.copilot",
  cursor: "stale.relogin.cursor",
  devin: "stale.relogin.devin",
  opencode: "stale.relogin.opencode",
  antigravity: "stale.relogin.antigravity",
  ollama: "stale.relogin.ollama",
  hermes: "stale.relogin.hermes",
  kimi: "stale.relogin.kimi",
};

/// The ⚠ Outdated tooltip: what went wrong, what fixes it, and the
/// reassurance that the visible numbers are the last good ones. Errors are
/// classified into sign-in / rate-limit / vendor-outage / connection
/// buckets so the fix is concrete instead of a bare HTTP code.
function staleHelp(s: Snapshot): string {
  const w = (s.warning ?? t("stale.lastFailed")).replace(/[.\s]+$/, "");
  const lw = w.toLowerCase();
  const manual = isManualQuotaProvider(s.id);
  const reloginKey = RELOGIN_KEYS[manual ? providerFamily(s.id) : s.id];
  const relogin = reloginKey ? t(reloginKey) : t("stale.reloginDefault");
  let fix = manual ? "" : t("stale.fixRetry");
  if (/run `|open the/.test(lw)) {
    // The provider's own message already says what to do.
    fix = t(manual ? "stale.manualDone" : "stale.fixDone");
  } else if (/http 40[13]|invalid_grant|expired|no refresh token|sign[- ]?in|log ?in|credentials/.test(lw)) {
    fix = t(manual ? "stale.manualRelogin" : "stale.fixRelogin", { how: relogin });
  } else if (/http 429|rate limit/.test(lw)) {
    fix = t(manual ? "stale.manual429" : "stale.fix429");
  } else if (/http 5\d\d/.test(lw)) {
    fix = t(manual ? "stale.manual5xx" : "stale.fix5xx");
  } else if (/error sending request|timed? ?out|connect|network|dns|proxy/.test(lw)) {
    fix = t("stale.fixNet");
  }
  if (manual) fix = [fix, t("card.manualHelp")].filter(Boolean).join("\n");
  return `${w}.\n${fix}\n${t("stale.tail")}`;
}

/// ⚠ shown when some events have no known model price — their tokens are
/// counted, but no dollars are guessed, so dollar totals under-report.
function unpricedWarn(sp: ProviderSpend | undefined): string {
  if (!sp || sp.unpriced <= 0) return "";
  const models = sp.unpriced_models.join(", ") || t("pricing.unknownModels");
  return `<span class="stale" title="${escapeHtml(
    t("unpriced.tip", { n: sp.unpriced, models }),
  )}">⚠</span>`;
}

type SpendTab = "today" | "yesterday" | "last30";

// Per-provider layout: which rows show, their order, which are tucked
// behind the caret ("On Demand"), and which are starred for the tray strip.
interface ProviderLayout {
  metricOrder: string[];
  onDemand: string[];
  hidden: string[];
  starred: string[];
  expanded: boolean;
  // One-shot: Bonus used to be a bar (always-visible). After the demotion
  // to a text row we tuck it once; later drags out of Show more stick.
  tuckedBonus?: boolean;
}

interface Layout {
  providerOrder: string[];
  providers: Record<string, ProviderLayout>;
}

interface AccessPolicy {
  version: number;
  enabledFamilies: string[];
  enabledAccounts: string[];
  scanRoots: Record<string, string[]>;
  scanEpochs?: Record<string, string>;
  scanSources?: Record<string, ScanSourceSettings>;
  regions: Record<string, string>;
  accountBindings: Record<string, { family: string; directory: string; name: string }>;
}

export interface Config {
  accessPolicy: AccessPolicy;
  refreshMinutes: number;
  providerAutoRefresh: Record<string, boolean>;
  disabled: string[];
  pinned: { provider: string; label: string } | null;
  trayProviders: string[];
  pacingAlways: boolean;
  notifyAlmostOut: boolean;
  notifyCuttingClose: boolean;
  notifyWillRunOut: boolean;
  notifyReset: boolean;
  spendTab: SpendTab;
  spendMetric: "cost" | "tokens" | "mtok";
  showUsed: boolean;
  resetExact: boolean;
  timeFormat: "auto" | "12" | "24";
  layout: Layout | null;
  appearance: "system" | "light" | "dark";
  density: "regular" | "compact";
  minimal: boolean;
  glassEffects: boolean;
  shortcut: string;
  proxy: { enabled: boolean; url: string };
  showTotalSpend: boolean;
  welcomeDismissed: boolean;
  lastSeenVersion: string;
  firstSeenMs: number;
  starPromptDone: boolean;
  starPromptDay: string;
  starPromptDayCount: number;
  starPromptLastMs: number;
  reduceAnimations: boolean;
  locale: LocalePref;
  stepfunPlanCredits: number | null;
  widgetMode: boolean;
  widgetCollapsed: boolean;
  widgetLocked: boolean;
  codexExtraDirs: string[];
}

const FRONTEND_CONFIG_KEYS = [
  "refreshMinutes",
  "disabled",
  "pinned",
  "trayProviders",
  "pacingAlways",
  "notifyAlmostOut",
  "notifyCuttingClose",
  "notifyWillRunOut",
  "notifyReset",
  "spendTab",
  "spendMetric",
  "showUsed",
  "resetExact",
  "timeFormat",
  "layout",
  "appearance",
  "density",
  "minimal",
  "glassEffects",
  "shortcut",
  "proxy",
  "showTotalSpend",
  "welcomeDismissed",
  "lastSeenVersion",
  "firstSeenMs",
  "starPromptDone",
  "starPromptDay",
  "starPromptDayCount",
  "starPromptLastMs",
  "reduceAnimations",
  "locale",
  "stepfunPlanCredits",
  "widgetMode",
  "widgetCollapsed",
  "widgetLocked",
  "codexExtraDirs",
] as const satisfies readonly (keyof Config)[];
type _AssertAllConfigKeys = Exclude<keyof Config, (typeof FRONTEND_CONFIG_KEYS)[number] | "accessPolicy" | "providerAutoRefresh"> extends never
  ? true
  : Exclude<keyof Config, (typeof FRONTEND_CONFIG_KEYS)[number] | "accessPolicy" | "providerAutoRefresh">;
const _assertAllConfigKeys: _AssertAllConfigKeys = true;
void _assertAllConfigKeys;

interface TrayProjectionProvider {
  metricOrder: string[];
  hidden: string[];
  starred: string[];
}

interface TrayProjectionConfig {
  disabled: string[];
  providerOrder: string[];
  providers: Record<string, TrayProjectionProvider>;
  pinned: Config["pinned"];
  locale: Locale;
}

interface UsageResult { revision: number; snapshots: Snapshot[]; }
let lastUsageRevision: number | null = null;

interface TrayStripEntry {
  id: string;
  logo: number[];
  labels: string[];
}

const ALL_PROVIDERS: [string, string][] = [
  ["claude", "Claude"],
  ["codex", "Codex"],
  ["cursor", "Cursor"],
  ["opencode", "OpenCode"],
  ["copilot", "Copilot"],
  ["grok", "Grok"],
  ["devin", "Devin"],
  ["minimax", "MiniMax"],
  ["openrouter", "OpenRouter"],
  ["zai", "Z.ai"],
  ["commandcode", "CommandCode"],
  ["antigravity", "Antigravity"],
  ["deepseek", "DeepSeek"],
  // Internal id stays "moonshot" (config/layout/telemetry compatibility);
  // the toggle reads "Kimi API" because that's what it gates: the API bar
  // on the Kimi card (or the standalone wallet card without a CLI login).
  ["moonshot", "Kimi API"],
  ["elevenlabs", "ElevenLabs"],
  ["ollama", "Ollama"],
  ["codebuff", "Codebuff"],
  ["kilo", "Kilo"],
  ["aihubmix", "AihubMix"],
  ["qwen", "Qwen Code"],
  ["hermes", "Hermes"],
  ["kimi", "Kimi Code"],
  ["stepfun", "StepFun"],
];

function providerDisplayName(id: string): string {
  return ALL_PROVIDERS.find(([pid]) => pid === id)?.[1] ?? id;
}

// Same quick links the Mac app ships (status pages + vendor dashboards).
const PROVIDER_LINKS: Record<string, { label: string; url: string }[]> = {
  claude: [
    { label: "Status", url: "https://status.anthropic.com/" },
    { label: "Dashboard", url: "https://claude.ai/settings/usage" },
  ],
  codex: [
    { label: "Status", url: "https://status.openai.com/" },
    { label: "Dashboard", url: "https://chatgpt.com/codex/settings/usage" },
  ],
  cursor: [
    { label: "Status", url: "https://status.cursor.com/" },
    { label: "Dashboard", url: "https://www.cursor.com/dashboard" },
  ],
  copilot: [
    { label: "Status", url: "https://www.githubstatus.com/" },
    { label: "Dashboard", url: "https://github.com/settings/billing" },
  ],
  grok: [
    { label: "Status", url: "https://status.x.ai" },
    { label: "Usage", url: "https://grok.com/?_s=usage" },
  ],
  devin: [{ label: "Dashboard", url: "https://app.devin.ai/settings/plans" }],
  minimax: [{ label: "Platform", url: "https://platform.minimax.io/" }],
  openrouter: [
    { label: "Activity", url: "https://openrouter.ai/activity" },
    { label: "Credits", url: "https://openrouter.ai/settings/credits" },
  ],
  zai: [
    { label: "Dashboard", url: "https://z.ai/manage-apikey/coding-plan/personal/my-plan" },
    { label: "API Keys", url: "https://z.ai/manage-apikey/apikey-list" },
    { label: "BigModel Keys", url: "https://open.bigmodel.cn/usercenter/apikeys" },
  ],
  commandcode: [
    { label: "Site", url: "https://commandcode.ai/" },
    { label: "Docs", url: "https://commandcode.ai/docs/resources/usage-limits" },
  ],
  opencode: [{ label: "Console", url: "https://opencode.ai/console" }],
  aihubmix: [{ label: "Console", url: "https://console.aihubmix.com/" }],
  qwen: [
    { label: "Coding Plan", url: "https://modelstudio.console.alibabacloud.com/ap-southeast-1/?tab=globalset#/efm/coding_plan" },
  ],
  deepseek: [
    { label: "Status", url: "https://status.deepseek.com/" },
    { label: "Platform", url: "https://platform.deepseek.com/usage" },
  ],
  moonshot: [{ label: "Console", url: "https://platform.moonshot.ai/console" }],
  elevenlabs: [
    { label: "Status", url: "https://status.elevenlabs.io/" },
    { label: "Usage", url: "https://elevenlabs.io/app/usage" },
  ],
  ollama: [{ label: "Library", url: "https://ollama.com/library" }],
  codebuff: [{ label: "Dashboard", url: "https://www.codebuff.com/profile" }],
  kilo: [{ label: "Dashboard", url: "https://app.kilo.ai/" }],
  hermes: [{ label: "Site", url: "https://hermes-agent.com/" }],
  kimi: [
    { label: "Console", url: "https://www.kimi.com/code/console" },
    { label: "Quota", url: "https://www.kimi.com/membership/subscription?tab=quota" },
    { label: "API", url: "https://platform.moonshot.ai/console" },
  ],
  stepfun: [
    { label: "Platform", url: "https://platform.stepfun.ai/" },
    { label: "Docs", url: "https://platform.stepfun.ai/docs/en/step-plan/overview" },
  ],
};

// Brand palette for the Total Spend ring (Mac parity); unknown providers
// get a stable hue derived from their id.
const SPEND_COLORS: Record<string, string> = {
  claude: "#de7356",
  codex: "#3b82f6",
  openrouter: "#6467f2",
  antigravity: "#4285f4",
  copilot: "#a855f7",
  minimax: "#f5433c",
  grok: "#10a37f",
  opencode: "#b7b1b1",
  devin: "#38bdf8",
  cursor: "var(--spend-cursor)", // brand black, theme-flipped in CSS
  moonshot: "#e0b354", // moon gold
  kimi: "#ff8a4c", // Kimi Code peach
  hermes: "#c2a878", // Nous tan
  aihubmix: "#5eead4", // hub teal
  qwen: "#8b5cf6", // Qwen violet
  __others__: "#8b8b94", // the folded small-spenders wedge
};

function spendColor(id: string): string {
  const fixed = SPEND_COLORS[id.replace(/^source:/, "")];
  if (fixed) return fixed;
  let hash = 0;
  for (const ch of id) hash = (hash * 31 + ch.charCodeAt(0)) >>> 0;
  return `hsl(${hash % 360} 62% 58%)`;
}

const SPEND_KEYS: [string, SpendTab][] = [
  ["Today", "today"],
  ["Yesterday", "yesterday"],
  ["Last 30 Days", "last30"],
];
const TREND_KEY = "Usage Trend";
const DIVIDER = "__ondemand__";

const STALE_MS = 60 * 1000;
let config: Config = {
  accessPolicy: { version: 1, enabledFamilies: [], enabledAccounts: [], scanRoots: {}, regions: {}, accountBindings: {} },
  refreshMinutes: 5,
  providerAutoRefresh: {},
  disabled: [],
  pinned: null,
  trayProviders: [],
  pacingAlways: false,
  notifyAlmostOut: false,
  notifyCuttingClose: false,
  notifyWillRunOut: false,
  notifyReset: false,
  spendTab: "today",
  spendMetric: "cost",
  showUsed: false,
  resetExact: false,
  timeFormat: "auto",
  layout: null,
  appearance: "system",
  density: "regular",
  minimal: false,
  glassEffects: true,
  shortcut: "",
  proxy: { enabled: false, url: "" },
  showTotalSpend: true,
  welcomeDismissed: false,
  lastSeenVersion: "",
  firstSeenMs: 0,
  starPromptDone: false,
  starPromptDay: "",
  starPromptDayCount: 0,
  starPromptLastMs: 0,
  reduceAnimations: false,
  locale: "auto",
  stepfunPlanCredits: null,
  widgetMode: false,
  widgetCollapsed: false,
  widgetLocked: false,
  codexExtraDirs: [],
};
let lastFetch = 0;
let refreshing = false;
// A forced refresh requested while one was already in flight (saving an
// API key races the auto-refresh timer). Dropping it would leave the new
// state unfetched and the status line stuck on the save message.
let refreshQueued = false;
let refreshQueuedUsageOnly = true;
type UsageRefreshReason = "automatic" | "userRefresh";
type UsageRefreshScope = { kind: "all" } | { kind: "account"; accountId: string };
let activeRefreshScope: UsageRefreshScope | null = null;
const refreshQueuedAccounts = new Map<string, number>();
const accountRefreshErrors = new Map<string, string>();
// A click's intent belongs to its access scope. Revoke/change that scope
// before the queued pass starts and a new explicit click is required.
let refreshQueuedManualEpoch: number | null = null;
let refreshGeneration = 0;
let completedRefreshGeneration = 0;
const refreshAttemptWaiters: Array<{ generation: number; resolve: () => void }> = [];
let lastAppliedSpendGen = 0;
let priceEpoch = 0;
let priceSync: Promise<void> | null = null;
let lastPriceSync: PricingStatus | null = null;
// Keep semantic text so locale changes preserve in-flight/results/error state.
let priceSyncStatus: { key: string; vars?: Record<string, string | number> } | null = null;
let refreshTimer: number | undefined;
let lastSnapshots: Snapshot[] = [];
let lastSpend: ProviderSpend[] = [];
let lastCursorSpendRevision: number | null = null;
let spendLoaded = false;
let spendTab: SpendTab = "today";
let customizeOpen = false;
let revealTimer = 0;
let animateExpandId: string | null = null;

/// One pass of entrance animations (cards slide in, bars fill) — played when
/// the popover opens or the first data lands, never on background re-renders.
function playReveal(): void {
  if (reduceMotion()) return;
  const el = document.querySelector<HTMLElement>("#providers");
  if (!el) return;
  el.classList.remove("reveal");
  void el.offsetWidth; // restart CSS animations
  el.classList.add("reveal");
  clearTimeout(revealTimer);
  revealTimer = window.setTimeout(() => el.classList.remove("reveal"), 950);
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

function escapeHtml(text: string): string {
  return text.replace(/[&<>"']/g, (c) => {
    const map: Record<string, string> = {
      "&": "&amp;",
      "<": "&lt;",
      ">": "&gt;",
      '"': "&quot;",
      "'": "&#39;",
    };
    return map[c];
  });
}

function clampPercent(value: number): number {
  return Math.min(100, Math.max(0, value));
}


function fmtMoney(v: number): string {
  if (v >= 1000) return `$${(v / 1000).toFixed(1)}K`;
  return `$${v.toFixed(2)}`;
}

function fmtTokens(v: number): string {
  if (v >= 1e9) return `${(v / 1e9).toFixed(1)}B`;
  if (v >= 1e6) return `${(v / 1e6).toFixed(1)}M`;
  if (v >= 1e3) return `${(v / 1e3).toFixed(1)}K`;
  return String(Math.round(v));
}

function fmtDuration(ms: number): string {
  const mins = Math.max(1, Math.round(ms / 60000));
  const days = Math.floor(mins / 1440);
  const hours = Math.floor((mins % 1440) / 60);
  const rem = mins % 60;
  if (days > 0) return t("time.daysHours", { d: days, h: hours });
  if (hours > 0) return t("time.hoursMins", { h: hours, m: String(rem).padStart(2, "0") });
  return t("time.mins", { m: rem });
}

// "today at 6:38 PM" / "tomorrow at 18:38" / "Sat, Jul 11 at 9:00 AM",
// honoring the Time Format setting.
function fmtExact(ts: number): string {
  const d = new Date(ts);
  const now = new Date();
  const hour12 =
    config.timeFormat === "12" ? true : config.timeFormat === "24" ? false : undefined;
  const tag = localeTag();
  const time = d.toLocaleTimeString(tag, { hour: "numeric", minute: "2-digit", hour12 });
  const dayStart = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const diffDays = Math.round((dayStart(d) - dayStart(now)) / 86400000);
  if (diffDays === 0) return t("time.today", { time });
  if (diffDays === 1) return t("time.tomorrow", { time });
  const date = d.toLocaleDateString(tag, { weekday: "short", month: "short", day: "numeric" });
  return t("time.dateAt", { date, time });
}

let configSaveQueue: Promise<void> = Promise.resolve();
let configSaveError: string | null = null;

function snapshotConfig(): Config {
  const payload = {} as Record<string, unknown>;
  for (const key of FRONTEND_CONFIG_KEYS) {
    payload[key] = config[key];
  }
  return JSON.parse(JSON.stringify(payload)) as Config;
}

function applyConfigEcho(sent: Config, echoed: Config): void {
  // Keep newer in-memory fields. Only take server canonicalization for
  // frontend keys that still match the snapshot this save actually wrote.
  const current = config as unknown as Record<string, unknown>;
  const from = sent as unknown as Record<string, unknown>;
  const echo = echoed as unknown as Record<string, unknown>;
  for (const key of FRONTEND_CONFIG_KEYS) {
    if (JSON.stringify(current[key]) === JSON.stringify(from[key])) {
      current[key] = echo[key];
    }
  }
}

async function patchConfig(patch: Partial<Config>): Promise<void> {
  Object.assign(config, patch);
  // Send a full current snapshot. If an earlier serialized write failed,
  // the next save retries that still-live in-memory state as well.
  const payload = snapshotConfig();
  const save = configSaveQueue.then(async () => {
    const echoed = await invoke<Config>("set_config", { patch: payload });
    applyConfigEcho(payload, echoed);
    if (!providerAutoRefreshUncertain) configSaveError = null;
  });
  configSaveQueue = save.catch(() => {});
  try {
    await save;
  } catch (err) {
    configSaveError = String(err);
    const status = document.querySelector("#status");
    if (status) status.textContent = t(providerAutoRefreshUncertain ? "footer.autoRefreshUnknown" : "footer.configSaveFailed", { err: configSaveError });
    throw err;
  }
}

// ---------------------------------------------------------------------------
// Layout: defaults, repair, persistence
// ---------------------------------------------------------------------------

function defaultProviderLayout(s: Snapshot | undefined, spend: ProviderSpend | undefined, migrateStar: boolean): ProviderLayout {
  const order: string[] = [];
  const onDemand: string[] = [];
  for (const m of s?.metrics ?? []) {
    if (order.includes(m.label)) continue; // one row per label
    order.push(m.label);
    // Used stays on the card: unlimited One/New API keys have no bar.
    if (m.kind !== "progress" && m.label !== "Used") onDemand.push(m.label);
  }
  // Balance-only providers (Moonshot, DeepSeek…) have no progress rows at
  // all — tucking everything would leave an empty card with a floating
  // caret, so their text rows stay visible.
  if (order.length > 0 && onDemand.length === order.length) onDemand.length = 0;
  if (spend) {
    order.push(TREND_KEY); // trend stays always-visible, like the Mac
    for (const [label] of SPEND_KEYS) {
      order.push(label);
      onDemand.push(label);
    }
  }
  const starred = migrateStar
    ? (s?.metrics ?? []).filter((m) => m.kind === "progress").slice(0, 2).map((m) => m.label)
    : [];
  return { metricOrder: order, onDemand, hidden: [], starred, expanded: false };
}




/// One/New API emits one quota row: Usage (limited bar), Used (unlimited),
/// or Limit. Switching unlimited↔limited must replace that slot so the
/// card never shows both 用量 and 已用.


function rankSnapshot(s: Snapshot): number {
  const FREE = /free|trial/i;
  if (s.status === "ok") {
    if (s.plan && !FREE.test(s.plan)) return 0;
    if (s.plan) return 2;
    return 1;
  }
  return s.status === "error" ? 3 : 4;
}

/// Builds the layout on first run and folds in newly-appeared providers or
/// metrics afterwards. Saves only when something actually changed.
function ensureLayout(): void {
  let changed = false;
  let layout = config.layout;

  if (!layout) {
    const orderedIds = [...lastSnapshots].sort((a, b) => rankSnapshot(a) - rankSnapshot(b)).map((s) => s.id);
    for (const [id] of ALL_PROVIDERS) if (!orderedIds.includes(id)) orderedIds.push(id);
    layout = { providerOrder: orderedIds, providers: {} };
    changed = true;
  }

  for (const [id] of ALL_PROVIDERS) {
    if (!layout.providerOrder.includes(id)) {
      layout.providerOrder.push(id);
      changed = true;
    }
  }
  // Configured One/New API keys keep an independent layout slot even
  // when the family is off (no snapshot). Append only — never regroup.

  // One-time label migration (Cursor bucket-era rename, 0.4.35): "Auto
  // usage" → "Cursor Models", "API usage" → "Other Models". Stars, pins,
  // hidden/on-demand flags and row order carry over — without this, a
  // starred/pinned old row silently loses its setting and the stale label
  // rots in metricOrder forever (no rename migration existed before).
  const CURSOR_RENAMES: Record<string, string> = {
    "Auto usage": "Cursor Models",
    "API usage": "Other Models",
  };
  for (const [pid, L] of Object.entries(layout.providers)) {
    if (providerFamily(pid) !== "cursor") continue;
    for (const list of [L.metricOrder, L.hidden, L.starred, L.onDemand]) {
      for (const [oldLabel, newLabel] of Object.entries(CURSOR_RENAMES)) {
        const at = list.indexOf(oldLabel);
        if (at < 0) continue;
        if (list.includes(newLabel)) list.splice(at, 1);
        else list[at] = newLabel;
        changed = true;
      }
    }
  }

  // MiniMax's rolling window is mcode's "5 Hours" row now. Same shape as
  // CURSOR_RENAMES: rename in place, splice out a surviving duplicate.
  const MINIMAX_RENAMES: Record<string, string> = { Session: "5 Hours" };
  for (const [pid, L] of Object.entries(layout.providers)) {
    if (providerFamily(pid) !== "minimax") continue;
    for (const list of [L.metricOrder, L.hidden, L.starred, L.onDemand]) {
      for (const [oldLabel, newLabel] of Object.entries(MINIMAX_RENAMES)) {
        const at = list.indexOf(oldLabel);
        if (at < 0) continue;
        if (list.includes(newLabel)) list.splice(at, 1);
        else list[at] = newLabel;
        changed = true;
      }
    }
  }
  if (config.pinned && providerFamily(config.pinned.provider) === "minimax") {
    const to = MINIMAX_RENAMES[config.pinned.label];
    if (to) {
      config.pinned = { ...config.pinned, label: to };
      void patchConfig({ pinned: config.pinned }).catch(() => {});
    }
  }

  // The per-credit "Reset credit"/"Reset credit N" rows collapsed into a
  // single "Rate Limit Resets" row. Same shape as CURSOR_RENAMES: the
  // first match is renamed in place (stars/order carry over), later
  // duplicates are spliced out.
  for (const [pid, L] of Object.entries(layout.providers)) {
    const family = providerFamily(pid);
    if (family !== "codex" && family !== "grok") continue;
    for (const list of [L.metricOrder, L.hidden, L.starred, L.onDemand]) {
      for (let i = 0; i < list.length; i++) {
        if (!/^Reset credits?(?: \d+)?$/.test(list[i])) continue;
        if (list.includes("Rate Limit Resets")) list.splice(i--, 1);
        else list[i] = "Rate Limit Resets";
        changed = true;
      }
    }
  }
  const hermesHasRecentModels = lastSnapshots.some(
    (s) => providerFamily(s.id) === "hermes" && s.metrics.some((m) => m.label === "Recent models"),
  );
  if (hermesHasRecentModels) {
    for (const [pid, L] of Object.entries(layout.providers)) {
      if (providerFamily(pid) !== "hermes") continue;
      for (const list of [L.metricOrder, L.hidden, L.starred, L.onDemand]) {
        const at = list.indexOf("Last used");
        if (at < 0) continue;
        if (list.includes("Recent models")) list.splice(at, 1);
        else list[at] = "Recent models";
        changed = true;
      }
    }
  }
  // On bucket-era accounts "Total usage" became a text row — the tray
  // strip and pinned tray number only accept progress metrics, so a
  // star/pin on it would silently vanish. Repoint both to the nearest
  // equivalent meter, "Cursor Models" (only when the live snapshot
  // confirms the row is text; pre-bucket accounts keep their bar).
  const cursorSnap = lastSnapshots.find((s) => providerFamily(s.id) === "cursor");
  const totalIsText =
    cursorSnap?.metrics.find((m) => m.label === "Total usage")?.kind === "text";
  if (totalIsText) {
    for (const [pid, L] of Object.entries(layout.providers)) {
      if (providerFamily(pid) !== "cursor") continue;
      const at = L.starred.indexOf("Total usage");
      if (at >= 0) {
        if (L.starred.includes("Cursor Models")) L.starred.splice(at, 1);
        else L.starred[at] = "Cursor Models";
        changed = true;
      }
    }
  }

    if (config.pinned && providerFamily(config.pinned.provider) === "cursor") {
    const renamed = CURSOR_RENAMES[config.pinned.label];
    const to = renamed ?? (totalIsText && config.pinned.label === "Total usage" ? "Cursor Models" : null);
    if (to) {
      config.pinned = { ...config.pinned, label: to };
      void patchConfig({ pinned: config.pinned }).catch(() => {});
    }
  }

  // "Bonus" briefly rendered as a bar and is now a text row (free
  // provider-sponsored usage — context, not a meter). Layouts saved in
  // that window placed it always-visible; tuck it behind Show more once,
  // then leave later Customize drags alone. Stars/pins on it still drop
  // every pass — the tray strip only accepts progress metrics.
  const bonusIsText =
    cursorSnap?.metrics.find((m) => m.label === "Bonus")?.kind === "text";
  if (bonusIsText) {
    for (const [pid, L] of Object.entries(layout.providers)) {
      if (providerFamily(pid) !== "cursor") continue;
      if (!L.tuckedBonus) {
        if (L.metricOrder.includes("Bonus") && !L.onDemand.includes("Bonus")) {
          L.onDemand.push("Bonus");
        }
        L.tuckedBonus = true;
        changed = true;
      }
      const starAt = L.starred.indexOf("Bonus");
      if (starAt >= 0) {
        L.starred.splice(starAt, 1);
        changed = true;
      }
    }
    if (
      config.pinned &&
      providerFamily(config.pinned.provider) === "cursor" &&
      config.pinned.label === "Bonus"
    ) {
      config.pinned = null;
      void patchConfig({ pinned: null }).catch(() => {});
    }
  }

  // Kimi Code folds the Moonshot wallet onto the plan card. Stars and the
  // tray pin on "Credits used" would otherwise vanish with that card —
  // but only migrate when the API bar is actually on that card, or we
  // plant a phantom star and the tray number goes blank.
  const kimiLive = lastSnapshots.some(
    (s) => s.id === "kimi" && s.status === "ok" && s.metrics.some((m) => m.label === "API"),
  );
  if (kimiLive) {
    const moonL = layout.providers.moonshot;
    const starAt = moonL?.starred.indexOf("Credits used") ?? -1;
    if (starAt >= 0 && moonL) {
      moonL.starred.splice(starAt, 1);
      let kimiL = layout.providers.kimi;
      if (!kimiL) {
        kimiL = defaultProviderLayout(
          lastSnapshots.find((s) => s.id === "kimi"),
          accountSpend("kimi"),
          false,
        );
        layout.providers.kimi = kimiL;
      }
      if (!kimiL.starred.includes("API")) {
        if (kimiL.starred.length >= 2) kimiL.starred.pop();
        kimiL.starred.push("API");
      }
      changed = true;
    }
    if (
      config.pinned?.provider === "moonshot" &&
      (config.pinned.label === "Credits used" || config.pinned.label === "API")
    ) {
      config.pinned = { provider: "kimi", label: "API" };
      void patchConfig({ pinned: config.pinned }).catch(() => {});
    }
  }

  for (const s of lastSnapshots) {
    if (!layout.providerOrder.includes(s.id)) {
      layout.providerOrder.push(s.id);
      changed = true;
    }
    const spend = accountSpend(s.id);
    let L = layout.providers[s.id];
    if (!L) {
      // One-time migration: providers picked in the old tray-strip setting
      // become starred so the strip carries over.
      L = defaultProviderLayout(s, spend, config.trayProviders.includes(s.id));
      layout.providers[s.id] = L;
      changed = true;
      continue;
    }


    // New metrics ship once; spend rows appear when spend data first exists.
    for (const m of s.metrics) {
      if (!L.metricOrder.includes(m.label)) {
        // Progress bars slot in above the Usage Trend (bars first, trend
        // after, like the Mac cards); everything else appends at the end.
        const trendAt = L.metricOrder.indexOf(TREND_KEY);
        const otherModelsAt = L.metricOrder.indexOf("Other Models");
        if (m.label === "Grok Bot" && otherModelsAt >= 0) {
          // One-off: Grok Bot belongs right after the "Other Models"
          // bucket bar — the spot a fresh layout gives it.
          L.metricOrder.splice(otherModelsAt + 1, 0, m.label);
        } else if (m.kind === "progress" && trendAt >= 0) {
          L.metricOrder.splice(trendAt, 0, m.label);
        } else {
          L.metricOrder.push(m.label);
        }
        if (m.kind !== "progress" && m.label !== "Used") L.onDemand.push(m.label);
        changed = true;
      }
      // Do not yank an existing progress row out of Show more or shuffle
      // it above Usage Trend on later refreshes. Extra credits flips
      // text↔progress with balance; a Customize drag would otherwise
      // bounce back on the next snapshot (issue #166). New rows still
      // land always-visible above the trend via the first-seen branch.
    }
    if (spend) {
      if (!L.metricOrder.includes(TREND_KEY)) {
        L.metricOrder.push(TREND_KEY);
        changed = true;
      }
      for (const [label] of SPEND_KEYS) {
        if (!L.metricOrder.includes(label)) {
          L.metricOrder.push(label);
          L.onDemand.push(label);
          changed = true;
        }
      }
    }
    // Repair layouts saved while a provider emitted duplicate labels (old
    // Grok billing bug): the label landed in metricOrder twice and the
    // card rendered the same row twice.
    const seenKeys = new Set<string>();
    const dedupedOrder = L.metricOrder.filter((k) => !seenKeys.has(k) && (seenKeys.add(k), true));
    if (dedupedOrder.length !== L.metricOrder.length) {
      L.metricOrder = dedupedOrder;
      changed = true;
    }
    // Repair saved layouts where EVERY visible row sits behind the caret
    // (balance-only cards defaulted that way before this rule existed):
    // an all-tucked card renders as an empty panel with a floating ⌄, so
    // its own metric rows are promoted back to always-visible.
    const alwaysVisible = L.metricOrder.filter(
      (k) => !L.onDemand.includes(k) && !L.hidden.includes(k),
    );
    // Partial Sub2API responses must not rewrite saved on-demand preferences.
    if (alwaysVisible.length === 0 && true) {
      const own = new Set(s.metrics.map((m) => m.label));
      if (s.metrics.length > 0 && L.onDemand.some((k) => own.has(k))) {
        L.onDemand = L.onDemand.filter((k) => !own.has(k));
        changed = true;
      }
    }
  }

  config.layout = layout;
  if (changed) void patchConfig({ layout });
}

function providerLayout(id: string): ProviderLayout {
  return (
    config.layout?.providers[id] ?? {
      metricOrder: [],
      onDemand: [],
      hidden: [],
      starred: [],
      expanded: false,
    }
  );
}

function liveProviderLayout(id: string): ProviderLayout {
  const layout = providerLayout(id);
  return layout;
}

function saveLayout(syncTray = true): void {
  if (!config.layout) return;
  // Undo history: remember the state we're moving away from.
  const next = JSON.stringify(config.layout);
  if (lastLayoutSnapshot && lastLayoutSnapshot !== next) {
    undoStack.push(lastLayoutSnapshot);
    if (undoStack.length > 50) undoStack.shift();
  }
  lastLayoutSnapshot = next;
  void patchConfig({ layout: config.layout });
  if (syncTray) requestTraySync();
}

// ---------------------------------------------------------------------------
// Pace engine (unchanged from Wave 1/4)
// ---------------------------------------------------------------------------

interface Pace {
  cls: string;
  note: string;
  noteClass: string;
  title: string;
  tick: number | null;
  /// Running out before reset — renderMetric draws a flame glyph.
  flame: boolean;
}

function computePace(m: Metric): Pace {
  const used = clampPercent(m.used_percent ?? 0);
  const left = 100 - used;
  const none: Pace = { cls: "", note: "", noteClass: "", title: "", tick: null, flame: false };

  if (left < 0.5) {
    return { cls: "low", note: t("pace.limitReached"), noteClass: "danger", title: t("pace.limitReachedTitle"), tick: null, flame: true };
  }

  const byLevel = (): Pace => {
    if (left <= 10) return { ...none, cls: "low", title: t("card.pctLeft", { n: Math.round(left) }) };
    if (used >= 80) return { ...none, cls: "warn", title: t("card.pctUsed", { n: Math.round(used) }) };
    return none;
  };
  if (!m.resets_at || !m.period_ms) return byLevel();

  const now = Date.now();
  const remainMs = Math.max(0, m.resets_at - now);
  const elapsedMs = m.period_ms - remainMs;
  const frac = elapsedMs / m.period_ms;
  if (frac < 0.05 || elapsedMs < 5 * 60000) return byLevel();
  // Near-empty windows stay calm: a floored 1% reading right at the
  // projection gate can land exactly on the limit and flash red (Mac
  // keeps the same 5% safeguard).
  if (used < 5) return byLevel();

  const projected = used / frac;
  const tick = clampPercent(frac * 100);

  if (projected >= 100) {
    const over = Math.round(projected - 100);
    const runOutAt = now + (left * elapsedMs) / used;
    if (runOutAt < m.resets_at - 60000) {
      const when = config.resetExact
        ? t("pace.limitAt", { when: fmtExact(runOutAt) })
        : t("pace.limitIn", { time: fmtDuration(runOutAt - now) });
      return { cls: "low", note: when, noteClass: "danger", title: t("pace.overReset", { n: over }), tick, flame: true };
    }
    return { cls: "low", note: "", noteClass: "danger", title: t("pace.fullReset"), tick, flame: true };
  }

  const spare = Math.max(1, Math.round(100 - projected));
  if (projected >= 90) {
    return {
      cls: "warn",
      note: t("pace.spare", { n: spare }),
      noteClass: "warn",
      title: t("pace.usedReset", { n: Math.round(projected) }),
      tick,
      flame: false,
    };
  }
  return {
    cls: "",
    note: config.pacingAlways ? t("pace.leftReset", { n: spare }) : "",
    noteClass: "",
    title: t("pace.leftReset", { n: spare }),
    tick: config.pacingAlways ? tick : null,
    flame: false,
  };
}

// ---------------------------------------------------------------------------
// Dashboard rendering
// ---------------------------------------------------------------------------

type ExpirySeverity = "normal" | "warning" | "critical";

/// Same bands as upstream's WidgetData.expirySeverity: critical within
/// 48h, warning within a week, normal beyond.
function expirySeverity(msRemaining: number): ExpirySeverity {
  if (msRemaining <= 48 * 3_600_000) return "critical";
  if (msRemaining <= 7 * 86_400_000) return "warning";
  return "normal";
}

/// Per-credit list carried in a "resets" row's detail; null when the count
/// came from a source without per-credit expiries (or the JSON is broken).
function parseResetCredits(m: Metric): ResetCredit[] | null {
  if (!m.detail) return null;
  try {
    const parsed = JSON.parse(m.detail);
    return Array.isArray(parsed) ? (parsed as ResetCredit[]) : null;
  } catch {
    return null;
  }
}

function renderMetric(m: Metric, providerId: string): string {
  if (m.kind === "progress" && m.used_percent !== null) {
    // An expiring credit past its deadline is dead — a stale/restored
    // snapshot can outlive resets_at, so render it fully gone rather
    // than trusting the last balance the API reported.
    const expired = m.expires === true && m.resets_at !== null && m.resets_at <= Date.now();
    const used = expired ? 100 : clampPercent(m.used_percent);
    const left = Math.round(100 - used);
    const pace: Pace = expired
      ? { cls: "low", note: "", noteClass: "", title: "", tick: null, flame: false }
      : computePace(m);
    const tick =
      pace.tick !== null && pace.tick > 1 && pace.tick < 99
        ? `<span class="tick" style="left:${pace.tick}%"></span>`
        : "";
    // The running-out glyph is drawn, not an emoji, so it matches the
    // note's color (danger red) and scales with the row's font.
    const flame = pace.flame
      ? `<svg class="pace-flame" viewBox="0 0 16 16" aria-hidden="true"><path fill="currentColor" d="M8 16c3.314 0 6-2 6-5.5 0-1.5-.5-4-2.5-6 .25 1.5-1.25 2-1.25 2C11 4 9 .5 6 0c.357 2 .5 4-2 6-1.25 1-2 2.729-2 4.5C2 14 4.686 16 8 16Zm0-1c-1.657 0-3-1-3-2.75 0-.75.25-2 1.25-3C6.125 10 7 10.5 7 10.5c-.375-1.25.5-3.25 2-3.5-.179 1-.25 2 1 3 .625.5 1 1.364 1 2.25C11 14 9.657 15 8 15Z"/></svg>`
      : "";
    const note = pace.flame || pace.note
      ? `<span class="pace-note ${pace.noteClass}" title="${escapeHtml(pace.title)}">${flame}${escapeHtml(pace.note)}</span>`
      : "";
    const headline = expired
      ? t("card.expired")
      : config.showUsed
        ? t("card.pctUsed", { n: Math.round(used) })
        : t("card.pctLeft", { n: left });
    const headlineAlt = expired
      ? t("card.expired")
      : config.showUsed
        ? t("card.pctLeft", { n: left })
        : t("card.pctUsed", { n: Math.round(used) });

    let resetHtml = "";
    if (expired) {
      resetHtml = `<span>${escapeHtml(t("card.expiredAt", { when: fmtExact(m.resets_at!) }))}</span>`;
    } else if (m.expires && m.resets_at !== null && m.resets_at > Date.now()) {
      // Expiring credit (e.g. Claude Cloud credits): the remaining value
      // dies at resets_at rather than refreshing — count down to the
      // loss, and never apply the notStarted grace (its clock doesn't
      // start on first use).
      const remain = m.resets_at - Date.now();
      const countdown = remain < 60_000 ? t("card.expiresSoon") : t("card.expiresIn", { time: fmtDuration(remain) });
      const exact = t("card.expires", { when: fmtExact(m.resets_at) });
      const [text, alt] = config.resetExact ? [exact, countdown] : [countdown, exact];
      resetHtml = `<span class="clickable" data-flip="reset" title="${escapeHtml(alt)}">${escapeHtml(text)}</span>`;
    } else if (m.resets_at !== null && m.resets_at > Date.now()) {
      // A rolling session window (≤6h period) that is still full-length
      // hasn't begun — its clock starts on the first message, so a
      // countdown would lie. Codex floors percentages and reports 1% on an
      // untouched window, so the label keys on the window being fresh
      // (with a grace for server-side reset staleness), not on a zero the
      // backend no longer fabricates.
      let notStarted = false;
      if (m.period_ms !== null && m.period_ms <= 6 * 3_600_000 && used <= 1) {
        const grace = Math.max(60_000, m.period_ms / 100);
        notStarted = m.resets_at - Date.now() >= m.period_ms - grace;
      }
      if (notStarted) {
        resetHtml = `<span title="${escapeHtml(t("card.notStartedTip"))}">${escapeHtml(t("card.notStarted"))}</span>`;
      } else {
        const remain = m.resets_at - Date.now();
        const countdown = remain < 60_000 ? t("card.resetsSoon") : t("card.resetsIn", { time: fmtDuration(remain) });
        const exact = t("card.resetsAt", { when: fmtExact(m.resets_at) });
        const [text, alt] = config.resetExact ? [exact, countdown] : [countdown, exact];
        resetHtml = `<span class="clickable" data-flip="reset" title="${escapeHtml(alt)}">${escapeHtml(text)}</span>`;
      }
    }
    const detailHtml = [
      expired || !m.detail ? "" : escapeHtml(displayMetricDetail(m.detail)),
      resetHtml,
    ].filter(Boolean).join(" · ");
    return `
      <div class="metric">
        <div class="metric-head">
          <span class="metric-label">${escapeHtml(displayMetricLabel(m.label))}</span>
          ${note}
        </div>
        <div class="bar" title="${escapeHtml(pace.title)}">
          <div class="fill ${pace.cls}" style="width:${used}%"></div>
          ${tick}
        </div>
        <div class="metric-foot">
          <span class="left-val clickable" data-flip="usage" title="${escapeHtml(headlineAlt)}">${headline}</span>
          <span class="detail">${detailHtml}</span>
        </div>
      </div>`;
  }
  // One row for all banked reset credits — count plus a severity dot off
  // the soonest expiry. The value is the hover target for the timeline
  // popover (Use → confirm → claim for Codex; read-only for Grok).
  if (m.kind === "resets") {
    const count = Number(m.value ?? 0) || 0;
    const credits = parseResetCredits(m);
    const soonest = credits
      ?.map((c) => c.expires_at)
      .filter((x): x is number => x !== null)
      .sort((a, b) => a - b)[0];
    const dot =
      count > 0 && soonest !== undefined
        ? `<span class="status-dot ${expirySeverity(soonest - Date.now())}"></span>`
        : "";
    return `
      <div class="metric-text resets-row">
        <span>${escapeHtml(displayMetricLabel(m.label))}</span>
        <span class="detail resets-value clickable" data-resets="${escapeHtml(providerId)}|${escapeHtml(m.label)}">${dot}${escapeHtml(t("card.nAvailable", { n: count }))}</span>
      </div>`;
  }
  // Action row (e.g. One/New API "Expiry"): exact expiry, and an amber dot
  // when a credit dies within 24h.
  if (m.kind === "action") {
    const expiry =
      m.resets_at !== null
        ? t("card.expires", { when: fmtExact(m.resets_at) })
        : displayMetricDetail(m.value ?? t("card.available"));
    const remaining = m.resets_at === null ? null : m.resets_at - Date.now();
    const soon =
      remaining !== null && remaining > 0 && remaining < 86_400_000
        ? `<span class="warn-dot" title="${escapeHtml(t("card.creditDying", { time: fmtDuration(remaining) }))}">●</span> `
        : "";
    return `
      <div class="metric-text action-row">
        <span>${soon}${escapeHtml(displayMetricLabel(m.label))}</span>
        <span class="action-right">
          <span class="detail">${escapeHtml(expiry)}</span>
        </span>
      </div>`;
  }
  // Weekly capacity (#236): a text row whose value chip opens the
  // per-week breakdown tooltip. The display value is rebuilt from the
  // detail JSON so it localizes; the backend's `value` is the fallback
  // (and what the local HTTP API serves).
  if (m.kind === "text" && m.label === "Weekly capacity") {
    const detail = parseCapacityDetail(m.detail);
    const text = detail ? capacityRowText(detail) : displayMetricDetail(m.value ?? "");
    return `
      <div class="metric-text">
        <span>${escapeHtml(displayMetricLabel(m.label))}</span>
        <span class="detail clickable" data-cap="${escapeHtml(providerId)}|${escapeHtml(m.label)}">${escapeHtml(text)}</span>
      </div>`;
  }
  return `
    <div class="metric-text">
      <span>${escapeHtml(displayMetricLabel(m.label))}</span>
      <span class="detail">${escapeHtml(displayMetricDetail(m.value ?? ""))}</span>
    </div>`;
}

function renderTrend(spend: ProviderSpend): string {
  if (!spend.trend.some((v) => v > 0)) return "";
  const max = Math.max(...spend.trend);
  const peakIdx = spend.trend.indexOf(max);
  const dayMs = 86_400_000;
  const dateOf = (i: number) =>
    new Date(Date.now() - (29 - i) * dayMs).toLocaleDateString(localeTag(), { month: "short", day: "numeric" });
  // Each day is a group: the visible bar plus a full-height invisible hit
  // area so thin bars are easy to hover; [data-trend] drives the tooltip.
  const slot = 128 / spend.trend.length;
  const bars = spend.trend
    .map((v, i) => {
      const h = v > 0 ? Math.max(2, (v / max) * 16) : 1;
      return `<g class="trend-day">
        <rect class="${v > 0 ? "trend-bar" : "trend-zero"}" x="${i * slot + slot / 4}" y="${18 - h}" width="${slot / 2}" height="${h}" rx="1"/>
        <rect class="trend-hit" data-trend="${escapeHtml(spend.id)}|${i}" x="${i * slot}" y="0" width="${slot}" height="18" fill="transparent"/>
      </g>`;
    })
    .join("");
  const title = t("spend.trendTip", {
    from: dateOf(0),
    to: dateOf(29),
    tokens: fmtTokens(max),
    peak: dateOf(peakIdx),
  });
  return `
    <div class="metric-text trend-row">
      <span title="${escapeHtml(title)}">${escapeHtml(t("spend.trend"))}</span>
      <svg class="trend-spark" viewBox="0 0 128 18" preserveAspectRatio="none">${bars}</svg>
    </div>`;
}

function renderSpendRow(
  providerId: string,
  label: string,
  key: SpendTab,
  w: SpendWindow,
  sp?: ProviderSpend,
): string {
  // Missing prices are unknown, even when the known-cost subtotal is zero.
  const cost = estimatedCostText(fmtMoney(w.cost), w.cost, sp?.unpriced ?? 0, t);
  const text =
    w.tokens > 0 || w.cost > 0.005
      ? providerId === "cursor"
        ? t("card.tokensEst", { cost, n: fmtTokens(w.tokens) })
        : t("card.tokensPlain", { cost, n: fmtTokens(w.tokens) })
      : t("card.noData");
  const warn = key === "last30" ? unpricedWarn(sp) : "";
  return `
    <div class="metric-text spend-row" data-spend="${escapeHtml(providerId)}|${key}">
      <span>${escapeHtml(displayMetricLabel(label))} ${warn}</span>
      <span class="detail">${text}</span>
    </div>`;
}

/// One card row addressed by its layout key. The key rides on the row's
/// root element as data-row so an in-card pointer drag can address it;
/// every renderer below returns a <div> root.
function renderItem(s: Snapshot, spend: ProviderSpend | undefined, key: string): string {
  let html: string;
  if (key === TREND_KEY) {
    html = spend ? renderTrend(spend) : "";
  } else {
    const spendKey = SPEND_KEYS.find(([label]) => label === key);
    if (spendKey) {
      html = spend ? renderSpendRow(s.id, spendKey[0], spendKey[1], spend[spendKey[1]], spend) : "";
    } else {
      const metric = s.metrics.find((m) => m.label === key);
      html = metric ? renderMetric(metric, s.id) : "";
    }
  }
  if (s.wallet_history && ["API", "Credits used", "Balance", "Vouchers", "Cash"].includes(key)) {
    const label = escapeHtml(displayMetricLabel(key));
    html = html.replace(label, `${label} <span class="wallet-saved">${escapeHtml(t("card.walletSavedRow"))}</span>`);
  }
  return html.replace("<div", `<div data-row="${escapeHtml(key)}"`);
}

/// Account-scoped cards (claude@<hash>) inherit their family's chrome —
/// icon, quick links — while keeping their own identity everywhere else.
function providerFamily(id: string): string {
  return id.split("@")[0];
}

// Match the backend's compatibility defaults and fail-closed malformed input.
// Settings belong to families; account-specific and unknown keys never opt in.
function normalizeProviderAutoRefresh(value: unknown): Record<string, boolean> {
  const record = value !== null && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown> : null;
  return Object.fromEntries(ALL_PROVIDERS.filter(([id]) => id !== "hermes").map(([id]) => [id,
    record && Object.prototype.hasOwnProperty.call(record, id) ? record[id] === true
      : value === undefined || record !== null ? !["claude", "commandcode"].includes(id) : false,
  ]));
}

function isManualQuotaProvider(id: string): boolean {
  const family = providerFamily(id);
  return family !== "hermes" && normalizeProviderAutoRefresh(config.providerAutoRefresh)[family] === false;
}

function renderProviderAutoRefresh(family: string): string {
  if (family === "hermes") return "";
  const checked = !isManualQuotaProvider(family);
  return `<div class="provider-auto-refresh"><label class="toggle"><input type="checkbox" data-provider-auto-refresh="${escapeHtml(family)}" aria-describedby="auto-refresh-help-${escapeHtml(family)}"${checked ? " checked" : ""} /> ${escapeHtml(t("platform.autoRefresh"))}</label><p id="auto-refresh-help-${escapeHtml(family)}" class="settings-note">${escapeHtml(t("platform.autoRefreshHelp"))}</p></div>`;
}

type ProviderAutoRefreshIntent = { family: string; enabled: boolean } | { reset: true };
let providerAutoRefreshVersion = 0;
let confirmedProviderAutoRefresh: Record<string, boolean> = {};
const providerAutoRefreshIntents = new Map<number, ProviderAutoRefreshIntent>();
let providerAutoRefreshUncertain = false;

function applyProviderAutoRefreshIntent(current: Record<string, boolean>, intent: ProviderAutoRefreshIntent): Record<string, boolean> {
  return "reset" in intent ? normalizeProviderAutoRefresh(undefined) : { ...current, [intent.family]: intent.enabled };
}

function renderProviderAutoRefreshState(): void {
  let current = providerAutoRefreshUncertain ? normalizeProviderAutoRefresh(null) : confirmedProviderAutoRefresh;
  for (const intent of providerAutoRefreshIntents.values()) current = applyProviderAutoRefreshIntent(current, intent);
  config.providerAutoRefresh = current;
  scheduleResetRefresh();
  renderProviderAccess();
  renderAll();
}

function changeProviderAutoRefresh(family: string, enabled: boolean): Promise<void> {
  if (family === "hermes" || !ALL_PROVIDERS.some(([id]) => id === family)) return Promise.resolve();
  return saveProviderAutoRefresh({ family, enabled });
}

function saveProviderAutoRefresh(intent: ProviderAutoRefreshIntent): Promise<void> {
  const version = ++providerAutoRefreshVersion;
  providerAutoRefreshIntents.set(version, intent);
  renderProviderAutoRefreshState();
  // Only this intent is applied when its turn arrives. A failed opt-in must
  // never be retried by a later unrelated family's captured optimistic map.
  // Ordinary layout/locale saves deliberately omit this dedicated field.
  const save = configSaveQueue.then(async () => {
    try {
      if (providerAutoRefreshUncertain) {
        const current = await invoke<Config>("get_config");
        confirmedProviderAutoRefresh = normalizeProviderAutoRefresh(current.providerAutoRefresh);
        providerAutoRefreshUncertain = false;
      }
      const next = "reset" in intent ? normalizeProviderAutoRefresh(undefined) : { [intent.family]: intent.enabled };
      const echoed = await invoke<Config>("set_config", { patch: { providerAutoRefresh: next } });
      confirmedProviderAutoRefresh = normalizeProviderAutoRefresh(echoed.providerAutoRefresh);
      if (version === providerAutoRefreshVersion) {
        configSaveError = null;
        document.querySelector("#status")!.textContent = t("footer.autoRefreshSaved");
      }
    } catch (err) {
      configSaveError = String(err);
      // Failed replies can follow committed writes. Canonical recovery is
      // serialized before later intents, and never replaces access policy.
      try {
        const echoed = await invoke<Config>("get_config");
        confirmedProviderAutoRefresh = normalizeProviderAutoRefresh(echoed.providerAutoRefresh);
        providerAutoRefreshUncertain = false;
      } catch {
        providerAutoRefreshUncertain = true;
      }
      document.querySelector("#status")!.textContent = t(providerAutoRefreshUncertain ? "footer.autoRefreshUnknown" : "footer.configSaveFailed", { err: configSaveError });
    } finally {
      providerAutoRefreshIntents.delete(version);
      renderProviderAutoRefreshState();
    }
  });
  configSaveQueue = save.catch(() => {});
  return save;
}

function experimentalProviderBadge(id: string): string {
  return providerFamily(id) === "commandcode"
    ? `<span class="experimental-badge" title="${escapeHtml(t("settings.commandcodeExperimental"))}">${escapeHtml(t("platform.experimental"))}</span>`
    : "";
}

/// One/New API is two-level: family id `onenewapi` hides every key card.
/// Claude/Codex extra accounts stay independent of the bare family id.
function isCardDisabled(id: string, disabled: string[] = config.disabled): boolean {
  if (disabled.includes(id)) return true;
  return false;
}

/// True when this layout key can actually paint a row right now.
function canRenderMinimal(s: Snapshot, spend: ProviderSpend | undefined, key: string): boolean {
  if (key === TREND_KEY) return Boolean(spend?.trend.some((v) => v > 0));
  if (SPEND_KEYS.some(([label]) => label === key)) return Boolean(spend);
  return s.metrics.some((m) => m.label === key);
}

/// The one row a card keeps in minimal view: a visible starred meter,
/// else the first visible progress meter, else the status word.
function minimalItemKey(s: Snapshot): string | null {
  const L =
    providerLayout(s.id);
  const spend = accountSpend(s.id);
  const visible = L.metricOrder.filter(
    (k) => !L.hidden.includes(k) && canRenderMinimal(s, spend, k),
  );
  const starred = L.starred.find((k) => visible.includes(k));
  if (starred) return starred;
  const progress = visible.find((k) =>
    s.metrics.some((m) => m.label === k && m.kind === "progress"),
  );
  if (progress) return progress;
  // Balance / credits / Used / spend rows have no progress meter.
  // Show that first visible value instead of collapsing the card to "ok".
  return visible[0] ?? null;
}

function canRefreshAccount(id: string): boolean {
  const binding = config.accessPolicy.accountBindings[id];
  const known = ALL_PROVIDERS.some(([provider]) => provider === id) ||
    (["claude", "codex", "opencode"].includes(providerFamily(id)) && binding?.family === providerFamily(id));
  return known && accountAuthorized(id) && !isCardDisabled(id) && lastSnapshots.some(snapshot => snapshot.id === id);
}

function accountRefreshBusy(id: string): boolean {
  return (activeRefreshScope?.kind === "account" && activeRefreshScope.accountId === id) || refreshQueuedAccounts.has(id);
}

function accountRefreshLabel(id: string, name: string): string {
  return t(accountRefreshBusy(id) ? "card.refreshAccountBusy" : "card.refreshAccount", { name });
}

function renderWalletHistory(s: Snapshot): string {
  if (!s.wallet_history) return "";
  const saved = s.wallet_history;
  const hasValues = s.metrics.some(metric => ["API", "Credits used", "Balance", "Vouchers", "Cash"].includes(metric.label));
  const label = t(hasValues ? "card.walletSaved" : "card.walletHistory");
  const time = saved.fetched_at != null
    ? t("card.walletLastSuccess", { time: new Date(saved.fetched_at).toLocaleString(localeTag()) })
    : t("card.walletTimeUnknown");
  // A failed first wallet query has history without values. Its attempt
  // clock is independent of both a wallet success and the fresh Kimi plan.
  const attempt = saved.attempted_at != null
    ? ` ${t("card.walletLastAttempt", { time: new Date(saved.attempted_at).toLocaleString(localeTag()) })}`
    : "";
  const warning = saved.warning ? ` ${saved.warning}` : "";
  const help = `${label}. ${time}${attempt} ${t("card.walletSavedHelp")}${warning}`;
  return `<p class="wallet-history" data-wallet-history title="${escapeHtml(help)}">${escapeHtml(label)} · ${escapeHtml(time + attempt)} ${escapeHtml(t("card.walletSavedHelp"))}${warning ? ` <span class="stale">${escapeHtml(warning)}</span>` : ""}</p>`;
}

function renderCard(s: Snapshot): string {
  const family = providerFamily(s.id);
  const manual = isManualQuotaProvider(s.id);
  const savedManual = manual && s.status === "ok" && s.stale;
  const manualHelp = [
    t("card.manualHelp"),
    ...(s.fetched_at != null ? [t("card.manualLastSuccess", { time: new Date(s.fetched_at).toLocaleString(localeTag()) })] : []),
    ...(savedManual ? [t("card.manualSavedIdentity")] : []),
  ].join("\n");
  const manualNotice = manual
    ? `<p class="manual-query" title="${escapeHtml(manualHelp)}"><span>${escapeHtml(t(savedManual ? "card.manualSaved" : "card.manualQuery"))}</span> · ${escapeHtml(t("card.manualHelp"))}</p>`
    : "";
  const planName = family === "commandcode" && s.plan === "GOAT · Experimental" ? "GOAT"
    : family === "commandcode" && s.plan === "Experimental" ? null : s.plan;
  const plan = planName ? `<span class="plan">${escapeHtml(planName)}</span>` : "";
  const icon = PROVIDER_ICONS[s.id] ?? PROVIDER_ICONS[providerFamily(s.id)] ?? "";
  const muted = s.status === "ok" ? "" : " muted";

  let body: string;
  let caret = "";
  if (s.status === "ok") {
    const L = providerLayout(s.id);
    const spend = accountSpend(s.id);
    if (config.minimal) {
      const key = minimalItemKey(s);
      body = key
        ? renderItem(s, spend, key)
        : `<p class="placeholder">${escapeHtml(s.status)}</p>`;
    } else {
      const visible = L.metricOrder.filter((k) => !L.hidden.includes(k));
      const always = visible.filter((k) => !L.onDemand.includes(k));
      const onDemand = visible.filter((k) => L.onDemand.includes(k));

      body = always.map((k) => renderItem(s, spend, k)).join("");
      const onDemandHtml = onDemand.map((k) => renderItem(s, spend, k)).join("");
      if (onDemandHtml.trim()) {
        const anim = L.expanded && animateExpandId === s.id ? " anim" : "";
        caret = `
        <button class="card-caret${L.expanded ? " expanded" : ""}" data-caret="${escapeHtml(s.id)}" title="${L.expanded ? t("card.showLess") : t("card.showMore")}"><svg class="caret-svg" viewBox="0 0 24 24" width="12" height="12" aria-hidden="true"><path d="M6 9l6 6 6-6" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"/></svg></button>
        ${L.expanded ? `<div class="on-demand${anim}">${onDemandHtml}</div>` : ""}`;
      }
    }
  } else if (s.status === "manual") {
    body = manual ? "" : `<p class="placeholder">${escapeHtml(t("card.autoPending"))}</p>`;
  } else {
    body = `<p class="placeholder">${escapeHtml(displayLocalSourceStatus(s.error ?? t("card.notConnected")))}</p>`;
  }

  // An automatic manual-only replay is a saved result, not a failed attempt.
  // Keep genuine failures visible without inventing a new failure message.
  // A fresh Kimi plan may coexist with saved wallet data. Its conservative
  // stale flag does not mean that the latest plan query failed.
  const savedWalletOnly = s.wallet_history && !s.attempt_failed &&
    (!s.warning || s.warning === s.wallet_history.warning || s.warning.startsWith("Moonshot API wallet"));
  const stale = s.stale && !(savedManual && !s.attempt_failed && !s.warning)
    ? savedWalletOnly
      ? `<span class="stale" title="${escapeHtml(t("card.walletSavedHelp"))}">${escapeHtml(t(s.metrics.some(metric => ["API", "Credits used", "Balance", "Vouchers", "Cash"].includes(metric.label)) ? "card.walletSaved" : "card.walletHistory"))}</span>`
      : `<span class="stale" title="${escapeHtml(staleHelp(s))}">${escapeHtml(t("card.outdated"))}</span>`
    : "";
  const dashUrl = (s.dashboard_url ?? "").trim();
  const dashOk = /^https?:\/\//i.test(dashUrl);
  const staticLinks = PROVIDER_LINKS[s.id] ?? PROVIDER_LINKS[family] ?? [];
  const linkItems = dashOk
    ? [{ label: "Dashboard", url: dashUrl }, ...staticLinks.filter((l) => l.label !== "Dashboard")]
    : staticLinks;
  const links = linkItems
    .filter((l) => l.label !== "API" || s.metrics.some((m) => m.label === "API"))
    .map((l) => `<button class="quick-link" data-link="${escapeHtml(l.url)}">${escapeHtml(displayLinkLabel(l.label))}</button>`)
    .join("<span class='quick-sep'>·</span>");
  const linksRow = config.minimal || !links ? "" : `<div class="quick-links">${links}</div>`;
  const share =
    !config.minimal && s.status === "ok"
      ? `<button class="share-btn" data-share="${escapeHtml(s.id)}" title="${escapeHtml(t("card.share"))}">⧉</button>`
      : "";
  const planChip = config.minimal ? "" : plan;
  const refreshLabel = escapeHtml(accountRefreshLabel(s.id, s.name));
  const accountRefresh = canRefreshAccount(s.id)
    ? `<button type="button" class="account-refresh" data-account-refresh="${escapeHtml(s.id)}" title="${refreshLabel}" aria-label="${refreshLabel}" aria-busy="${accountRefreshBusy(s.id)}"${accountRefreshBusy(s.id) ? " disabled" : ""}>↻</button>`
    : "";
  const quotaWarning = family === "commandcode" && s.status === "ok" && !s.stale && s.warning
    ? `<p class="quota-warning" data-quota-warning role="status">${escapeHtml(s.warning)}</p>`
    : "";
  const refreshError = accountRefreshErrors.get(s.id);
  const refreshNotice = refreshError ? `<p class="account-refresh-error" role="status">${escapeHtml(t("card.refreshAccountFailed", { err: refreshError }))}</p>` : "";
  return `
    <article class="provider${muted}" data-provider="${escapeHtml(s.id)}">
      <div class="provider-head">
        <span class="provider-icon drag-handle" title="${escapeHtml(t("card.drag"))}">${icon || '<span class="grip-glyph">⠿</span>'}</span>
        <span class="provider-name">${escapeHtml(s.name)}</span>
        ${experimentalProviderBadge(s.id)}
        ${planChip}
        ${stale}
        <span class="spacer"></span>
        ${accountRefresh}
        ${share}
      </div>
      <div class="card-panel">
        ${manualNotice}
        ${quotaWarning}
        ${refreshNotice}
        ${renderWalletHistory(s)}
        ${body}
        ${linksRow}
        ${caret}
      </div>
    </article>`;
}

function orderedSnapshots(): Snapshot[] {
  const order = config.layout?.providerOrder ?? [];
  // Disabled providers disappear immediately — not on the next fetch.
  return lastSnapshots.filter((s) => accountAuthorized(s.id) && !isCardDisabled(s.id)).sort((a, b) => {
    const ia = order.indexOf(a.id);
    const ib = order.indexOf(b.id);
    if (ia !== -1 && ib !== -1) return ia - ib;
    return rankSnapshot(a) - rankSnapshot(b);
  });
}

// The ring is built from annular wedges (like the Mac's SectorMark chart):
// radial-cut ends with softly rounded corners and angular gaps, so tiny
// spenders stay thin slivers instead of ballooning to a round-cap dot.
const TAU = Math.PI * 2;
const DONUT_OUT = 44; // outer radius
const DONUT_IN = 30; // inner radius — 14 thick, centered on r=37
const DONUT_PAD = 2.2 / 37; // angular gap between neighbors (~2px mid-ring)
const DONUT_MIN = 0.07; // slimmest visible sliver (~2.6px mid-ring)

type DonutEntry = {
  s: ProviderSpend;
  w: SpendWindow;
  unpriced: number;
  /// Present on the synthetic "Others" entry: the folded-in providers,
  /// largest first, for the hover breakdown.
  parts?: { name: string; w: SpendWindow; unpriced: number }[];
  /// The dollar bar the parts fell under (period-specific).
  foldLimit?: number;
};

const OTHERS_ID = "__others__";
/// Providers under this many dollars (in the visible window) fold into
/// one "Others" wedge; hovering it lists who spent what. The bar scales
/// with the period — a day's ring earns a slice at $5, a month's at $10.
function othersFoldUsd(tab: SpendTab): number {
  return tab === "last30" ? 10 : 5;
}

function spendVisible(row: ProviderSpend): boolean {
  return scanSpendVisible(row, config.accessPolicy.scanRoots, accountAuthorized("cursor") && !isCardDisabled("cursor"));
}
function accountSpend(id: string): ProviderSpend | undefined {
  return lastSpend.find((row) => row.id === id && row.sources.length === 0 && spendVisible(row));
}
function localSpendName(row: ProviderSpend): string {
  if (row.id === "source:claude") return t("local.claudeModelUsage");
  if (row.id === "source:codex") return t("local.codexModelUsage");
  return row.name.replace(/ logs$/, "");
}
function renderLocalSpendCards(): string {
  return lastSpend.filter((row) => row.sources.length > 0 && spendVisible(row)).map((row) => {
    const sourceNames = row.sources.map((source) => SCAN_SOURCES.find(([id]) => id === source)?.[1] ?? source).join(", ");
    const details = row.local_stats.map((metric) => `<div class="metric-text"><span>${escapeHtml(displayMetricLabel(metric.label))}</span><span>${escapeHtml(displayLocalStatValue(metric.value))}</span></div>`).join("");
    const totals = SPEND_KEYS.map(([label, key]) => renderSpendRow(row.id, label, key, row[key], row)).join("");
    return `<article class="provider local-spend" data-local-source="${escapeHtml(row.id)}"><div class="provider-head"><span class="provider-name">${escapeHtml(localSpendName(row))}</span><span class="plan">${escapeHtml(t("local.logs"))}</span></div><div class="card-panel"><p class="placeholder">${escapeHtml(t("local.sourceCaveat", { names: sourceNames }))}</p>${details}${totals}${renderTrend(row)}</div></article>`;
  }).join("");
}

function donutEntries(tab: SpendTab): DonutEntry[] {
  const all: DonutEntry[] = lastSpend
    .filter(spendVisible)
    .map((s) => ({ s: s.sources.length ? { ...s, name: t("local.donutName", { name: localSpendName(s) }) } : s, w: s[tab], unpriced: s.unpriced }))
    // Membership, order, and wedge share all follow the active metric so
    // the legend ranking always matches the ring (cost keeps a half-cent
    // noise floor). Unknown-cost usage remains in the legend even when it
    // has no known dollars to draw as a wedge.
    .filter((e) =>
      config.spendMetric === "tokens"
        ? e.w.tokens > 0
        : config.spendMetric === "mtok"
          ? e.w.tokens > 0 && (e.w.cost > 0.005 || e.unpriced > 0)
          : e.w.cost > 0.005 || (e.w.tokens > 0 && e.unpriced > 0),
    )
    .sort((a, b) => spendVal(b.w) - spendVal(a.w));

  // Small spenders fold into a single "Others" wedge — even a lone one,
  // so under-threshold providers never claim their own legend row. Only
  // exception: at least one named provider must remain, because an
  // all-Others ring says nothing.
  const limit = othersFoldUsd(tab);
  const small = all.filter((e) => e.w.cost < limit);
  if (small.length === 0 || small.length === all.length) return all;

  const others: DonutEntry = {
    s: {
      id: OTHERS_ID,
      name: t("spend.others"),
    } as ProviderSpend,
    w: {
      cost: small.reduce((sum, e) => sum + e.w.cost, 0),
      tokens: small.reduce((sum, e) => sum + e.w.tokens, 0),
      models: [],
    },
    unpriced: small.reduce((sum, e) => sum + e.unpriced, 0),
    parts: small.map((e) => ({ name: e.s.name, w: e.w, unpriced: e.unpriced })),
    foldLimit: limit,
  };
  return [...all.filter((e) => e.w.cost >= limit), others].sort(
    (a, b) => spendVal(b.w) - spendVal(a.w),
  );
}

/// The donut meters dollars or raw tokens — a click on the ring toggles.
function spendVal(w: SpendWindow): number {
  if (config.spendMetric === "tokens") return w.tokens;
  if (config.spendMetric === "mtok") return w.tokens > 0 ? w.cost / (w.tokens / 1e6) : 0;
  return w.cost;
}

/// Dollar-rate figure: two decimals under $1k, abbreviated above.
function fmtRate(v: number): string {
  return v < 1000 ? `$${v.toFixed(2)}` : fmtMoney(v);
}

/// The ring's two-line center (and its hover text) for the active metric.
/// Cost/MTok is the overall average — total dollars over total megatokens —
/// not a sum of per-provider rates.
function spendCenter(entries: DonutEntry[]): { primary: string; sub: string; exact: string } {
  const missing = lastSpend.filter(spendVisible).some((row) => row.unpriced > 0);
  if (config.spendMetric === "mtok") {
    const cost = entries.reduce((s, e) => s + e.w.cost, 0);
    const mtok = entries.reduce((s, e) => s + e.w.tokens, 0) / 1e6;
    const rate = mtok > 0 ? cost / mtok : 0;
    return missing ? { primary: t("pricing.unknown"), sub: t("pricing.missingPrices"), exact: t("pricing.unknownAverage") } : { primary: fmtRate(rate), sub: "$/MTok", exact: `${fmtRate(rate)}/MTok average` };
  }
  if (config.spendMetric === "tokens") {
    const tokens = entries.reduce((s, e) => s + e.w.tokens, 0);
    return { primary: fmtTokens(tokens), sub: t("spend.centerTokens"), exact: t("card.tokens", { n: fmtTokens(tokens) }) };
  }
  const c = entries.reduce((s, e) => s + e.w.cost, 0);
  return { primary: missing ? (c > 0 ? `${fmtMoney(c)}+` : t("pricing.unknown")) : fmtMoney(c), sub: missing ? t("pricing.knownCostsOnly") : t("spend.metric.cost"), exact: missing ? t("pricing.unknownTotal") : `$${c.toFixed(2)}` };
}

/// The metric a click (or right-click, reversed) moves to next — the Mac
/// menu's order: Cost, Cost/MTok, Tokens.
function nextSpendMetric(back: boolean): "cost" | "tokens" | "mtok" {
  const order: ("cost" | "tokens" | "mtok")[] = ["cost", "mtok", "tokens"];
  const i = order.indexOf(config.spendMetric);
  return order[(i + (back ? order.length - 1 : 1)) % order.length];
}

const METRIC_NAMES = { cost: "spend.metric.cost", mtok: "spend.metric.mtok", tokens: "spend.metric.tokens" } as const;

function fmtSpendVal(w: SpendWindow, unpriced: number): string {
  if (config.spendMetric === "tokens") return fmtTokens(w.tokens);
  if (config.spendMetric === "mtok") return unpriced > 0 ? t("pricing.unknown") : `${fmtRate(spendVal(w))}/MTok`;
  return estimatedCostText(fmtMoney(w.cost), w.cost, unpriced, t);
}

/// Angular extent per provider (slivers lifted to stay visible), shared by
/// the initial render and the tab-switch morph. Angles run clockwise from
/// 12 o'clock; the first gap straddles the top like the Mac's ring.
function donutGeometry(entries: DonutEntry[]): { total: number; geo: Map<string, { a0: number; a1: number }> } {
  // An unknown average has no numeric angle. Keep its legend entry and its
  // exact tokens, but exclude it (including mixed Others) from rate geometry.
  const measured = config.spendMetric === "mtok" ? entries.filter((e) => e.unpriced <= 0) : entries;
  const total = measured.reduce((sum, e) => sum + spendVal(e.w), 0);
  const spenders = measured.filter((e) => spendVal(e.w) > 0);
  const geo = new Map<string, { a0: number; a1: number }>();
  if (spenders.length === 0 || total <= 0) return { total, geo };
  if (spenders.length === 1) {
    geo.set(spenders[0].s.id, { a0: 0, a1: TAU });
    return { total, geo };
  }
  const avail = TAU - spenders.length * DONUT_PAD;
  const spans = spenders.map((e) => (spendVal(e.w) / total) * avail);
  let excess = 0;
  for (let i = 0; i < spans.length; i++) {
    if (spans[i] < DONUT_MIN) {
      excess += DONUT_MIN - spans[i];
      spans[i] = DONUT_MIN;
    }
  }
  if (excess > 0) {
    const big = spans.indexOf(Math.max(...spans));
    spans[big] = Math.max(DONUT_MIN, spans[big] - excess);
  }
  let a = DONUT_PAD / 2;
  spenders.forEach((e, i) => {
    geo.set(e.s.id, { a0: a, a1: a + spans[i] });
    a += spans[i] + DONUT_PAD;
  });
  return { total, geo };
}

function donutPt(r: number, a: number): string {
  return `${(48 + r * Math.sin(a)).toFixed(2)} ${(48 - r * Math.cos(a)).toFixed(2)}`;
}

/// SVG path for one annular sector with rounded corners (d3-arc style).
/// A full-circle span comes back as a two-ring evenodd annulus instead.
function sectorPath(a0: number, a1: number): string {
  const span = a1 - a0;
  if (span >= TAU - 0.0001) {
    const ring = (r: number, sweep: number) =>
      `M ${donutPt(r, 0)} A ${r} ${r} 0 1 ${sweep} ${donutPt(r, Math.PI)} A ${r} ${r} 0 1 ${sweep} ${donutPt(r, TAU)} Z`;
    return `${ring(DONUT_OUT, 1)} ${ring(DONUT_IN, 0)}`;
  }
  // Corner radius shrinks on thin slivers so the roundings never overlap.
  const s = Math.sin(span / 2);
  const rc = Math.max(
    0.2,
    Math.min(3, (DONUT_OUT - DONUT_IN) / 2, (DONUT_IN * s) / (1 - s), (DONUT_OUT * s) / (1 + s)),
  );
  const f1 = Math.asin(rc / (DONUT_OUT - rc)); // angle eaten by an outer corner
  const f0 = Math.asin(rc / (DONUT_IN + rc)); // …and by an inner corner
  const d1 = Math.sqrt((DONUT_OUT - rc) ** 2 - rc * rc); // corner tangents on the radial cuts
  const d0 = Math.sqrt((DONUT_IN + rc) ** 2 - rc * rc);
  return [
    `M ${donutPt(d1, a0)}`,
    `A ${rc} ${rc} 0 0 1 ${donutPt(DONUT_OUT, a0 + f1)}`,
    `A ${DONUT_OUT} ${DONUT_OUT} 0 ${span - 2 * f1 > Math.PI ? 1 : 0} 1 ${donutPt(DONUT_OUT, a1 - f1)}`,
    `A ${rc} ${rc} 0 0 1 ${donutPt(d1, a1)}`,
    `L ${donutPt(d0, a1)}`,
    `A ${rc} ${rc} 0 0 1 ${donutPt(DONUT_IN, a1 - f0)}`,
    `A ${DONUT_IN} ${DONUT_IN} 0 ${span - 2 * f0 > Math.PI ? 1 : 0} 0 ${donutPt(DONUT_IN, a0 + f0)}`,
    `A ${rc} ${rc} 0 0 1 ${donutPt(d0, a0)}`,
    "Z",
  ].join(" ");
}

/// Hover nudges a wedge outward along its bisector, Mac-style.
function donutPop(g: { a0: number; a1: number }): { tx: string; ty: string } {
  const mid = (g.a0 + g.a1) / 2;
  return { tx: `${(2.5 * Math.sin(mid)).toFixed(2)}px`, ty: `${(-2.5 * Math.cos(mid)).toFixed(2)}px` };
}

/// Hover text for the "Others" wedge/row: who's inside and what each spent.
function othersBreakdown(e: DonutEntry): string {
  if (!e.parts) return "";
  return (
    (e.unpriced > 0 ? `${t("pricing.knownCostsOnly")}\n` : "") +
    `${t("spend.underEach", { limit: e.foldLimit ?? 1 })}\n` +
    e.parts.map((p) => `${p.name}  ${fmtSpendVal(p.w, p.unpriced)}`).join("\n")
  );
}

function legendHtml(entries: DonutEntry[]): string {
  return entries
    .map(
      (e) => `
        <div class="legend-row" data-pid="${escapeHtml(e.s.id)}"${e.parts ? ` title="${escapeHtml(othersBreakdown(e))}"` : ""}>
          <span class="dot" style="background:${spendColor(e.s.id)}"></span>
          <span class="legend-name">${escapeHtml(e.s.name)}</span>
          <span class="legend-val">${escapeHtml(fmtSpendVal(e.w, e.unpriced))}</span>
        </div>`,
    )
    .join("");
}

/// Tab switch morphs the existing arcs in place (identity-keyed per
/// provider, CSS-transitioned) instead of rebuilding the card.
function switchSpendTab(tab: SpendTab): void {
  spendTab = tab;
  void patchConfig({ spendTab });
  const card = document.querySelector<HTMLElement>(".total-spend");
  const paths = card ? Array.from(card.querySelectorAll<SVGPathElement>("path.seg")) : [];
  const entries = donutEntries(tab);
  const { geo } = donutGeometry(entries);
  // Wedge paths share one command structure so CSS can tween `d`; a
  // full-circle annulus doesn't, so single-spender states rebuild instead.
  const morphable =
    card &&
    paths.length > 0 &&
    geo.size >= 2 &&
    paths.every((p) => !p.dataset.full) &&
    [...geo.keys()].every((id) => paths.some((p) => p.dataset.pid === id));
  if (!morphable) {
    renderAll();
    return;
  }
  const entryById = new Map(entries.map((en) => [en.s.id, en]));
  for (const p of paths) {
    const g = geo.get(p.dataset.pid ?? "");
    if (g) {
      const pop = donutPop(g);
      p.style.opacity = "1";
      p.style.setProperty("d", `path("${sectorPath(g.a0, g.a1)}")`);
      p.style.setProperty("--tx", pop.tx);
      p.style.setProperty("--ty", pop.ty);
    } else {
      p.style.opacity = "0";
    }
    // The Others wedge bakes its breakdown into an SVG <title>; the legend
    // rebuilds below but this child wouldn't, so sync it to the new period
    // (and drop it from any wedge that no longer carries a breakdown).
    const en = entryById.get(p.dataset.pid ?? "");
    const text = en?.parts ? othersBreakdown(en) : "";
    const t = p.querySelector("title");
    if (text) {
      if (t) {
        t.textContent = text;
      } else {
        const nt = document.createElementNS("http://www.w3.org/2000/svg", "title");
        nt.textContent = text;
        p.appendChild(nt);
      }
    } else if (t) {
      t.remove();
    }
  }
  const totalEl = card.querySelector(".donut-total");
  const center = spendCenter(entries);
  if (totalEl) totalEl.textContent = center.primary;
  const legend = card.querySelector(".legend");
  if (legend) legend.innerHTML = legendHtml(entries);
  card.querySelectorAll(".tab").forEach((t) => {
    t.classList.toggle("active", t.getAttribute("data-tab") === tab);
  });
  const wrap = card.querySelector<HTMLElement>(".donut-wrap");
  if (wrap) {
    wrap.title = t("spend.clickTip", {
      exact: center.exact,
      next: t(`spend.metric.${nextSpendMetric(false)}`),
    });
  }
}

function renderTotalSpend(): string {
  if (!config.showTotalSpend) return "";
  const entries = donutEntries(spendTab);
  if (lastSpend.length === 0) {
    // Quiet state instead of a missing card — on a fresh PC the donut only
    // appears after a CLI (Claude Code, Codex, Grok…) has logged some usage.
    const note = spendLoaded ? t("spend.emptyFirst") : t("spend.scanning");
    return `
      <article class="provider total-spend">
        <div class="provider-head">
          <span class="provider-name">${escapeHtml(t("spend.title"))}</span>
        </div>
        <div class="card-panel"><p class="placeholder" style="margin:4px 0">${note}</p></div>
      </article>`;
  }

  const { geo } = donutGeometry(entries);
  const segments = entries
    .filter((e) => geo.has(e.s.id))
    .map((e) => {
      const g = geo.get(e.s.id)!;
      const pop = donutPop(g);
      const full = g.a1 - g.a0 >= TAU - 0.0001 ? ` data-full="1"` : "";
      const hint = e.parts ? `<title>${escapeHtml(othersBreakdown(e))}</title>` : "";
      return `<path class="seg" data-pid="${escapeHtml(e.s.id)}"${full} fill-rule="evenodd"
        d="${sectorPath(g.a0, g.a1)}" style="fill:${spendColor(e.s.id)};--tx:${pop.tx};--ty:${pop.ty}">${hint}</path>`;
    })
    .join("");

  const legend = legendHtml(entries);

  const tab = (id: SpendTab, label: string) =>
    `<button class="tab${spendTab === id ? " active" : ""}" data-tab="${id}">${label}</button>`;

  const center = spendCenter(entries);
  const exact = t("spend.clickTip", {
    exact: center.exact,
    next: t(METRIC_NAMES[nextSpendMetric(false)]),
  });
  // An empty window still draws the ring — a zeroed track with $0.00 in the
  // center — so the card doesn't collapse to bare text between periods.
  const body = entries.length
    ? `
      <div class="donut-wrap" title="${escapeHtml(exact)}">
        <svg width="96" height="96" viewBox="0 0 96 96">
          ${segments}
          <text class="donut-total" x="48" y="50" text-anchor="middle" font-size="14" font-weight="600">${center.primary}</text>
          <text class="donut-sub" x="48" y="62" text-anchor="middle" font-size="8">${center.sub}</text>
        </svg>
        <div class="legend">${legend}</div>
      </div>`
    : `
      <div class="donut-wrap donut-empty" title="${escapeHtml(t("spend.emptyPeriodTip"))}">
        <svg width="96" height="96" viewBox="0 0 96 96">
          <path class="seg donut-zero" data-full="1" fill-rule="evenodd" d="${sectorPath(0, TAU)}"/>
          <text class="donut-total" x="48" y="50" text-anchor="middle" font-size="14" font-weight="600">${center.primary}</text>
          <text class="donut-sub" x="48" y="62" text-anchor="middle" font-size="8">${center.sub}</text>
        </svg>
        <div class="legend"><p class="placeholder" style="margin:0">${escapeHtml(t("spend.emptyPeriod"))}</p></div>
      </div>`;

  const contributors = lastSpend.map((s) => s.name).join(", ");
  return `
    <article class="provider total-spend">
      <div class="provider-head">
        <span class="provider-name">${escapeHtml(t("spend.title"))}</span>
        <span class="info" title="${escapeHtml(t("spend.info", { names: contributors }))}">&#9432;</span>
        <span class="spacer"></span>
        <button class="share-btn" data-share="__total__" title="${escapeHtml(t("card.share"))}">⧉</button>
      </div>
      <div class="card-panel">
        <div class="tabs">
          ${tab("today", t("spend.today"))}${tab("yesterday", t("spend.yesterday"))}${tab("last30", t("spend.days30"))}
        </div>
        ${body}
      </div>
    </article>`;
}

// ---------------------------------------------------------------------------
// Footer update flow — check on launch and every popover open; the backend
// also checks at launch and every 4 h. The version stamp becomes "Checking
// for updates…" and then an Update button on a hit.
// ---------------------------------------------------------------------------

let buildText = "";
function renderBuildInfo(): void {
  const el = document.querySelector<HTMLElement>("#build-info");
  if (el) el.textContent = buildText;
}

// ---------------------------------------------------------------------------
// Share cards — the live card element rasterized to PNG on the clipboard
// ---------------------------------------------------------------------------

/// Copy a card exactly as it appears on screen: serialize the live card
/// element plus the app stylesheet into an SVG <foreignObject> and
/// rasterize it at 2x. Whatever the card renders — donut, tabs, trend
/// bars, future rows — the copied image matches automatically, instead
/// of a hand-drawn approximation that drifts from the real UI.
/// In-app replacement for window.confirm: the native dialog renders as a
/// bare "localhost says" browser popup, which has no place in a glass UI.
/// Resolves true on confirm; Esc, the ✕, backdrop clicks, and Cancel all
/// resolve false. The keydown listener runs in the capture phase and stops
/// propagation so the app's global Esc (close panels) stays out of it.
/// Cancels the open appConfirm dialog, if any. The popover hides on focus
/// loss with the dialog still in the DOM — reopening must not resurface a
/// stale question, so the reopen routine dismisses it like Esc would.
let dismissConfirm: (() => void) | null = null;

function appConfirm(opts: {
  title: string;
  message: string;
  confirmLabel: string;
  danger?: boolean;
}): Promise<boolean> {
  return new Promise((resolve) => {
    const overlay = document.createElement("div");
    overlay.id = "confirm-overlay";
    overlay.innerHTML = `
      <div id="confirm-box" role="dialog" aria-modal="true">
        <h3>${escapeHtml(opts.title)}</h3>
        <p>${escapeHtml(opts.message)}</p>
        <div id="confirm-actions">
          <button id="confirm-cancel" type="button">${escapeHtml(t("dialog.cancel"))}</button>
          <button id="confirm-ok" type="button" class="${opts.danger ? "danger" : ""}">${escapeHtml(opts.confirmLabel)}</button>
        </div>
      </div>`;
    const done = (ok: boolean) => {
      dismissConfirm = null;
      document.removeEventListener("keydown", onKey, true);
      overlay.remove();
      resolve(ok);
    };
    dismissConfirm = () => done(false);
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        done(false);
      }
    };
    overlay.addEventListener("click", (e) => {
      if (e.target === overlay) done(false);
    });
    overlay.querySelector("#confirm-cancel")!.addEventListener("click", () => done(false));
    overlay.querySelector("#confirm-ok")!.addEventListener("click", () => done(true));
    document.addEventListener("keydown", onKey, true);
    document.body.appendChild(overlay);
    overlay.querySelector<HTMLButtonElement>("#confirm-ok")!.focus();
  });
}

// ---------------------------------------------------------------------------
// Changelog — "What's new" after an update + the Settings viewer
// ---------------------------------------------------------------------------

interface ChangelogSection {
  version: string;
  date: string;
  body: string;
}

/// CHANGELOG.md split into per-version sections, newest first. The
/// "Unreleased" section is skipped — a shipped build's own notes carry its
/// version header (release retitles Unreleased), so users only ever see
/// released entries.
function parseChangelog(): ChangelogSection[] {
  const sections: ChangelogSection[] = [];
  for (const block of changelogRaw.split(/^## /m).slice(1)) {
    const nl = block.indexOf("\n");
    const header = block.slice(0, nl).trim();
    if (/^unreleased$/i.test(header)) continue;
    const m = header.match(/^([\d.]+)\s*—\s*(.+)$/);
    sections.push({
      version: m ? m[1] : header,
      date: m ? m[2] : "",
      body: block.slice(nl + 1).trim(),
    });
  }
  return sections;
}

/// Markdown-lite for changelog bodies: ### subheads, - bullets (with hanging
/// continuation lines), plain paragraphs, **bold**, `code`. Bullets and
/// paragraphs accumulate as raw markdown and are transformed only on flush,
/// so a bold/code span wrapped across the file's ~70-column lines still
/// matches. Input is escaped before any markup is applied, so the changelog
/// can never inject HTML.
function renderChangelogBody(md: string): string {
  const inline = (s: string) =>
    escapeHtml(s)
      .replace(/\*\*(.+?)\*\*/g, "<strong>$1</strong>")
      .replace(/`([^`]+)`/g, "<code>$1</code>");
  let html = "";
  let items: string[] = [];
  let para = "";
  const flushItems = () => {
    if (items.length) html += `<ul>${items.map((i) => `<li>${inline(i)}</li>`).join("")}</ul>`;
    items = [];
  };
  const flushPara = () => {
    if (para) html += `<p>${inline(para)}</p>`;
    para = "";
  };
  for (const line of md.split("\n")) {
    if (line.startsWith("### ")) {
      flushItems();
      flushPara();
      html += `<h5>${escapeHtml(line.slice(4).trim())}</h5>`;
    } else if (line.startsWith("- ")) {
      flushPara();
      items.push(line.slice(2));
    } else if (/^\s+\S/.test(line) && items.length) {
      items[items.length - 1] += " " + line.trim();
    } else if (line.trim()) {
      flushItems();
      para += (para ? " " : "") + line.trim();
    } else {
      flushPara();
    }
  }
  flushItems();
  flushPara();
  return html;
}

/// Same lifecycle as dismissConfirm: the popover reopen routine clears a
/// stale dialog left behind by hide-on-focus-loss.
let dismissWhatsNew: (() => void) | null = null;

/// Card-styled scrollable dialog listing changelog sections. Esc, backdrop
/// clicks (anywhere outside the card), and the Got it button all dismiss.
function showChangelogDialog(title: string, sections: ChangelogSection[]): void {
  dismissWhatsNew?.();
  const overlay = document.createElement("div");
  overlay.id = "whatsnew-overlay";
  const list = sections
    .map(
      (s) =>
        `<section><h4>v${escapeHtml(s.version)}${
          s.date ? `<span>${escapeHtml(s.date)}</span>` : ""
        }</h4>${renderChangelogBody(s.body)}</section>`,
    )
    .join("");
  overlay.innerHTML = `
    <div id="whatsnew-box" role="dialog" aria-modal="true">
      <h3>${escapeHtml(title)}</h3>
      <div id="whatsnew-body">${list}</div>
      <div id="whatsnew-actions">
        <button id="whatsnew-ok" type="button">${escapeHtml(t("dialog.gotIt"))}</button>
      </div>
    </div>`;
  const done = () => {
    dismissWhatsNew = null;
    document.removeEventListener("keydown", onKey, true);
    overlay.remove();
  };
  dismissWhatsNew = done;
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      done();
    }
  };
  overlay.addEventListener("click", (e) => {
    if (e.target === overlay) done();
  });
  overlay.querySelector("#whatsnew-ok")!.addEventListener("click", done);
  document.addEventListener("keydown", onKey, true);
  document.body.appendChild(overlay);
}

/// The sections a just-updated install hasn't seen yet (newest first,
/// capped), or null when there's nothing to announce. Marks the current
/// version as seen immediately so the dialog can only ever appear once per
/// version, even if it's dismissed by closing the popover.
let appVersion = "";
let pendingWhatsNew: ChangelogSection[] | null = null;

function computeWhatsNew(version: string): ChangelogSection[] | null {
  const last = config.lastSeenVersion;
  if (last === version) return null;
  void patchConfig({ lastSeenVersion: version });
  const all = parseChangelog();
  if (!last) {
    // First run with this feature. An install that already dismissed the
    // welcome card is an *update* — show the new version's notes. A true
    // fresh install gets the welcome card instead, not two popups. Guard
    // the empty case (e.g. a build whose notes are still Unreleased) —
    // an empty array is truthy and would present a blank dialog.
    const own = config.welcomeDismissed ? all.filter((s) => s.version === version) : [];
    return own.length ? own : null;
  }
  const out: ChangelogSection[] = [];
  for (const s of all) {
    if (s.version === last || out.length >= 5) break;
    out.push(s);
  }
  return out.length ? out : null;
}

// ---------------------------------------------------------------------------
// Star prompt — asks for a GitHub star, at most twice a day
// ---------------------------------------------------------------------------

/// Same lifecycle as dismissConfirm/dismissWhatsNew: the popover reopen
/// routine clears a stale prompt left behind by hide-on-focus-loss.
let dismissStarPrompt: (() => void) | null = null;

/// Local YYYY-MM-DD for the "twice a day" cap — day boundaries follow the
/// user's clock, not UTC.
function starPromptToday(): string {
  const d = new Date();
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/// A roll that won but hasn't presented yet. An interrupted winning roll
/// records nothing — hiding before the timer fires just wastes the roll.
let starPromptTimer: number | undefined;

/// Eligibility + the random roll: the install must be a few days old, at
/// most twice a day, never within four hours of the last show. Winning
/// only arms the timer — the counters commit in presentStarPrompt, when
/// the dialog is actually appended (a prompt inserted into a hidden
/// webview would burn the budget unseen).
function maybeShowStarPrompt(): void {
  const now = Date.now();
  const today = starPromptToday();
  if (
    config.starPromptDone ||
    !config.firstSeenMs ||
    now - config.firstSeenMs < 3 * 86_400_000 ||
    now - config.starPromptLastMs < 4 * 3_600_000 ||
    (config.starPromptDay === today && config.starPromptDayCount >= 2)
  ) {
    return;
  }
  if (Math.random() >= 0.25) return;
  if (starPromptTimer !== undefined) return; // a roll is already pending
  // After the reveal animation; the guards are re-checked inside.
  starPromptTimer = window.setTimeout(() => {
    starPromptTimer = undefined;
    presentStarPrompt();
  }, 450);
}

/// Small glass dialog: Star on GitHub retires it forever (and opens the
/// repo), "Don't ask again" retires it, Maybe later / Esc / backdrop just
/// close.
function presentStarPrompt(): void {
  // A dialog (or a second prompt) may have presented while the reveal
  // played — never stack. And a hidden window can't see it at all.
  if (document.hidden || dismissConfirm || dismissWhatsNew || dismissStarPrompt) return;
  const today = starPromptToday();
  void patchConfig({
    starPromptLastMs: Date.now(),
    starPromptDay: today,
    starPromptDayCount:
      config.starPromptDay === today ? config.starPromptDayCount + 1 : 1,
  }).catch(() => {});
  const overlay = document.createElement("div");
  overlay.id = "star-overlay";
  overlay.innerHTML = `
    <div id="star-box" role="dialog" aria-modal="true">
      <div class="star-glyph">★</div>
      <h3>${escapeHtml(t("star.title"))}</h3>
      <p>${escapeHtml(t("star.body"))}</p>
      <div id="star-actions">
        <button id="star-go" type="button" class="primary">${escapeHtml(t("star.go"))}</button>
        <button id="star-later" type="button">${escapeHtml(t("star.later"))}</button>
      </div>
      <button id="star-never" type="button" class="linkish">${escapeHtml(t("star.never"))}</button>
    </div>`;
  const done = () => {
    dismissStarPrompt = null;
    document.removeEventListener("keydown", onKey, true);
    overlay.remove();
  };
  dismissStarPrompt = done;
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      done();
    }
  };
  const retire = () => {
    void patchConfig({ starPromptDone: true }).catch(() => {});
    done();
  };
  overlay.addEventListener("click", (e) => {
    if (e.target === overlay) done();
  });
  overlay.querySelector("#star-later")!.addEventListener("click", done);
  overlay.querySelector("#star-never")!.addEventListener("click", retire);
  overlay.querySelector("#star-go")!.addEventListener("click", () => {
    void patchConfig({ starPromptDone: true }).catch(() => {});
    void invoke("open_link", { url: "https://github.com/ItsJazii/pane" }).catch((err) => {
      document.querySelector("#status")!.textContent = t("footer.openLinkFailed", { err: String(err) });
    });
    done();
  });
  document.addEventListener("keydown", onKey, true);
  document.body.appendChild(overlay);
}

async function shareCard(id: string): Promise<void> {
  const status = document.querySelector("#status")!;
  try {
    const el =
      id === "__total__"
        ? document.querySelector<HTMLElement>("article.total-spend")
        : document.querySelector<HTMLElement>(`article.provider[data-provider="${id}"]`);
    if (!el) return;

    const rect = el.getBoundingClientRect();
    const W = Math.ceil(rect.width);
    const S = 2;
    const PAD = 16; // frame around the card, like the Mac share cards
    const FOOT = 34; // logo + tagline row
    // The tagline row already carries its own breathing room, so the frame
    // under it is thin — otherwise the tagline floats with dead space below.
    const PAD_BOTTOM = 4;

    let css = "";
    for (const sheet of Array.from(document.styleSheets)) {
      try {
        for (const rule of Array.from(sheet.cssRules)) css += rule.cssText + "\n";
      } catch {
        // Inaccessible sheet (shouldn't happen — all styles are bundled).
      }
    }
    // Static rasterization renders CSS animations at time zero, which for
    // the entrance animations means an invisible card. Freeze final state.
    // The body's inherited text styles are re-declared on the wrapper since
    // the snapshot document has no <body>.
    const bodyStyle = getComputedStyle(document.body);
    css +=
      "*{animation:none!important;transition:none!important}" +
      "#snap-root .share-btn{display:none!important}" +
      `#snap-foot{display:flex;align-items:center;justify-content:center;gap:6px;` +
      `height:${FOOT}px;color:var(--muted-foreground);font-size:12px}` +
      "#snap-foot img{width:16px;height:16px;border-radius:4px}";

    const clone = el.cloneNode(true) as HTMLElement;
    clone.style.margin = "0";
    clone.style.width = `${W}px`;
    clone.style.boxSizing = "border-box";

    // Shares are strictly what's on screen: everything the card currently
    // renders — bars, pace hints, the trend, and the On Demand section
    // when it's open — copies as-is. Only interactive chrome (buttons,
    // links, carets, grips) never belongs in an image. (The old "compact
    // composition" for collapsed cards is retired: it dropped the visible
    // trend and pace hints, which read as missing data in the copy.)
    // .snap-card restores the card surface the popover no longer draws
    // (cards sit flat on the background there, panels carry the chrome).
    clone.classList.add("snap-card");
    if (id !== "__total__") {
      clone
        .querySelectorAll(".share-btn, .card-caret, .quick-links, .action-row, .drag-grip, .grip-glyph")
        .forEach((n) => n.remove());
    }

    // The curated clone is shorter than the on-screen card (chrome
    // removed), so measure IT — briefly attached offscreen — instead of
    // sizing the canvas from the original and leaving dead space.
    clone.style.position = "fixed";
    clone.style.left = "-99999px";
    clone.style.top = "0";
    document.body.appendChild(clone);
    const H = Math.ceil(clone.getBoundingClientRect().height);
    clone.remove();
    clone.style.position = "";
    clone.style.left = "";
    clone.style.top = "";
    const W2 = W + PAD * 2;
    const H2 = H + PAD + FOOT + PAD_BOTTOM;
    css +=
      `#snap-root{font-family:${bodyStyle.fontFamily};font-size:${bodyStyle.fontSize};` +
      `color:${bodyStyle.color};letter-spacing:${bodyStyle.letterSpacing};` +
      `background:var(--background);padding:${PAD}px ${PAD}px ${PAD_BOTTOM}px;box-sizing:border-box;` +
      `width:${W2}px;height:${H2}px}`;

    // data-theme / data-density live on <html>; :root of the snapshot
    // document is the <svg>, so the attributes are mirrored there for the
    // :root[data-…] rules to keep matching.
    const root = document.documentElement;
    const svgMarkup =
      `<svg xmlns="http://www.w3.org/2000/svg" width="${W2 * S}" height="${H2 * S}" ` +
      `viewBox="0 0 ${W2} ${H2}" data-theme="${root.dataset.theme ?? ""}" ` +
      `data-density="${root.dataset.density ?? ""}" ` +
      `data-minimal="${root.dataset.minimal ?? "false"}">` +
      `<foreignObject width="${W2}" height="${H2}">` +
      `<div xmlns="http://www.w3.org/1999/xhtml" id="snap-root">` +
      // CDATA so CSS containing XML-special characters (`<`, `&` — e.g. in
      // a content: string) can never malform the snapshot document. A
      // literal "]]>" inside CSS would end the section early, so split it.
      `<style><![CDATA[${css.split("]]>").join("]]]]><![CDATA[>")}]]></style>` +
      new XMLSerializer().serializeToString(clone) +
      `<div id="snap-foot"><img src="${paneIcon}" alt="" /><span>${escapeHtml(t("share.tagline"))}</span></div>` +
      `</div></foreignObject></svg>`;

    const img = new Image();
    img.src = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svgMarkup)}`;
    await img.decode();

    const canvas = document.createElement("canvas");
    canvas.width = W2 * S;
    canvas.height = H2 * S;
    const ctx = canvas.getContext("2d")!;
    ctx.drawImage(img, 0, 0);

    const dataUrl = canvas.toDataURL("image/png");
    const pngBase64 = dataUrl.slice(dataUrl.indexOf(",") + 1);
    await invoke("copy_share_image", { pngBase64 });
    status.textContent = t("footer.copied");
  } catch (err) {
    status.textContent = t("footer.shareFailed", { err: String(err) });
  }
}

// ---------------------------------------------------------------------------
// Liquid glass lens (prasen.dev original). A rounded-rect signed-distance
// field drives the displacement map, so refraction is concentrated at the
// rim while the center stays optically flat — like iOS Liquid Glass.
// ---------------------------------------------------------------------------

function generateLensMap(w: number, h: number): string | null {
  const canvas = document.createElement("canvas");
  canvas.width = w;
  canvas.height = h;
  const ctx = canvas.getContext("2d");
  if (!ctx) return null;
  const img = ctx.createImageData(w, h);
  const data = img.data;
  const cx = w / 2;
  const cy = h / 2;
  const radius = Math.min(w, h) / 2;
  const halfW = Math.max(w / 2 - radius, 0);
  const halfH = Math.max(h / 2 - radius, 0);
  const rim = 1.1 * radius; // bend zone width, measured inward from the edge
  let i = 0;
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const ax = x + 0.5 - cx;
      const ay = y + 0.5 - cy;
      const px = Math.abs(ax) - halfW;
      const py = Math.abs(ay) - halfH;
      const sdf =
        Math.min(Math.max(px, py), 0) + Math.hypot(Math.max(px, 0), Math.max(py, 0)) - radius;
      let g = 0;
      if (sdf > -rim) {
        const e = Math.min(Math.max(1 + sdf / rim, 0), 1);
        g = e * e * (3 - 2 * e); // smoothstep toward the edge
      }
      data[i++] = Math.round(128 + (ax / (w / 2)) * g * 110);
      data[i++] = Math.round(128 + (ay / (h / 2)) * g * 110);
      data[i++] = 128;
      data[i++] = 255;
    }
  }
  ctx.putImageData(img, 0, 0);
  return canvas.toDataURL();
}

function applyLens(el: HTMLElement | null, filterId: string, imgId: string): void {
  if (!el) return;
  const w = 4 * Math.round(el.offsetWidth / 4);
  const h = 4 * Math.round(el.offsetHeight / 4);
  if (w < 8 || h < 8) return;
  const filter = document.getElementById(filterId);
  const img = document.getElementById(imgId);
  const map = generateLensMap(w, h);
  if (!filter || !img || !map) return;
  filter.setAttribute("width", String(w));
  filter.setAttribute("height", String(h));
  img.setAttribute("width", String(w));
  img.setAttribute("height", String(h));
  img.setAttribute("href", map);
  const f = `url(#${filterId}) blur(2px) saturate(1.8) brightness(1.04)`;
  el.style.backdropFilter = f;
  (el.style as unknown as Record<string, string>).webkitBackdropFilter = f;
}

/// "Liquid glass effects" off swaps the SDF refraction + backdrop blurs
/// for flat surfaces (body.no-glass CSS overrides win over the inline
/// styles applyLens sets). The expensive displacement filters then never
/// run — the fix for laptops where the popover animates below 60 fps.
function applyGlass(): void {
  document.body.classList.toggle("no-glass", config.glassEffects === false);
  // Lens init is skipped entirely while glass is off — build the maps the
  // first time the user turns it on.
  if (config.glassEffects !== false && !lensReady) initLiquidLens();
}

function reduceMotion(): boolean {
  return (
    config.reduceAnimations === true ||
    window.matchMedia("(prefers-reduced-motion: reduce)").matches
  );
}

function applyReduceMotion(): void {
  document.body.classList.toggle("reduce-anim", config.reduceAnimations === true);
}

let lensReady = false;

function initLiquidLens(): void {
  if (config.glassEffects === false || lensReady) return;
  lensReady = true;
  const surfaces: [string, string, HTMLElement | null][] = [
    ["lens-side", "lens-map-side", document.querySelector(".sidebar")],
    ["lens-footer", "lens-map-footer", document.querySelector(".main-col footer")],
  ];
  for (const [filterId, imgId, el] of surfaces) {
    if (!el) continue;
    applyLens(el, filterId, imgId);
    new ResizeObserver(() => applyLens(el, filterId, imgId)).observe(el);
  }

  // Panel header bars (Customize / Settings) share one lens sized to the
  // window width. Applied through a CSS variable so re-rendered bars keep
  // the effect without JS re-application.
  const w = 4 * Math.round(window.innerWidth / 4);
  const h = 44;
  const filter = document.getElementById("lens-bar");
  const img = document.getElementById("lens-map-bar");
  const map = generateLensMap(w, h);
  if (filter && img && map) {
    filter.setAttribute("width", String(w));
    filter.setAttribute("height", String(h));
    img.setAttribute("width", String(w));
    img.setAttribute("height", String(h));
    img.setAttribute("href", map);
    document.documentElement.style.setProperty(
      "--bar-filter",
      "url(#lens-bar) blur(2px) saturate(1.8) brightness(1.04)",
    );
  }
}

// ---------------------------------------------------------------------------
// Appearance (System / Light / Dark) + density (Regular / Compact)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Tooltip bubbles: every `title` attribute is silently upgraded to a custom
// bubble — 400ms deliberate dwell, balanced wrapping, anchored to the item.
// ---------------------------------------------------------------------------

function setupTooltips(): void {
  const tip = document.createElement("div");
  tip.id = "hover-tip";
  tip.hidden = true;
  document.body.appendChild(tip);
  let timer = 0;
  let anchor: HTMLElement | null = null;

  const hide = () => {
    clearTimeout(timer);
    tip.hidden = true;
    anchor = null;
  };

  document.addEventListener("mouseover", (e) => {
    // The collapsed widget is too short for the bubble — keep the native
    // tooltip there, which the OS draws outside the window.
    if (document.body.classList.contains("widget-collapsed")) return;
    const el = (e.target as HTMLElement).closest<HTMLElement>("[title], [data-tip]");
    if (!el) return;
    const title = el.getAttribute("title");
    if (title) {
      el.dataset.tip = title;
      el.removeAttribute("title"); // suppress the native tooltip
    }
    if (!el.dataset.tip || el === anchor) return;
    anchor = el;
    clearTimeout(timer);
    timer = window.setTimeout(() => {
      if (anchor !== el || !document.contains(el)) return;
      tip.textContent = el.dataset.tip ?? "";
      tip.hidden = false;
      const r = el.getBoundingClientRect();
      const w = tip.offsetWidth;
      const h = tip.offsetHeight;
      const x = Math.max(6, Math.min(r.left + r.width / 2 - w / 2, window.innerWidth - w - 6));
      let y = r.top - h - 8;
      if (y < 6) y = r.bottom + 8;
      tip.style.left = `${x}px`;
      tip.style.top = `${y}px`;
    }, 400);
  });
  document.addEventListener("mouseout", (e) => {
    const el = (e.target as HTMLElement).closest<HTMLElement>("[data-tip]");
    const to = e.relatedTarget as HTMLElement | null;
    if (el && (!to || !el.contains(to))) hide();
  });
  document.addEventListener("scroll", hide, true);
  document.addEventListener("mousedown", hide, true);
}

// ---------------------------------------------------------------------------
// Customize undo — whole-layout snapshots, Ctrl+Z restores.
// ---------------------------------------------------------------------------

const undoStack: string[] = [];
let lastLayoutSnapshot = "";

function undoLayout(): void {
  const prev = undoStack.pop();
  if (!prev) return;
  config.layout = JSON.parse(prev) as Layout;
  lastLayoutSnapshot = prev;
  void patchConfig({ layout: config.layout });
  renderAll();
  requestTraySync();
  document.querySelector("#status")!.textContent = "Layout change undone";
}

// ---------------------------------------------------------------------------
// Party mode 🎉 — ↑↑↓↓←→←→BA. Purely cosmetic, never persisted.
// ---------------------------------------------------------------------------

const KONAMI = [
  "ArrowUp", "ArrowUp", "ArrowDown", "ArrowDown",
  "ArrowLeft", "ArrowRight", "ArrowLeft", "ArrowRight", "b", "a",
];
let konamiAt = 0;

function toggleParty(): void {
  const on = document.body.classList.toggle("party");
  document.querySelector("#status")!.textContent = on ? "🎉 Party mode!" : "Party's over.";
}

function konamiListen(e: KeyboardEvent): void {
  const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
  konamiAt = key === KONAMI[konamiAt] ? konamiAt + 1 : key === KONAMI[0] ? 1 : 0;
  if (konamiAt === KONAMI.length) {
    konamiAt = 0;
    toggleParty();
  }
}

const systemLight = window.matchMedia("(prefers-color-scheme: light)");

function applyAppearance(): void {
  const mode =
    config.appearance === "system" ? (systemLight.matches ? "light" : "dark") : config.appearance;
  document.documentElement.dataset.theme = mode;
  document.documentElement.dataset.density = config.density;
  document.documentElement.dataset.minimal = config.minimal ? "true" : "false";
  const minimal = document.querySelector<HTMLInputElement>("#minimal");
  if (minimal) minimal.checked = config.minimal === true;
  const btn = document.querySelector<HTMLElement>("#theme-btn");
  if (btn) {
    btn.textContent = mode === "light" ? "☾" : "☀";
    btn.title = mode === "light" ? t("sidebar.themeToDark") : t("sidebar.themeToLight");
    delete btn.dataset.tip;
  }
}

/// Day/night toggle with the circular wipe from jazii.dev: the new theme
/// expands as a clip-path circle from the button via the View Transitions
/// API. Falls back to an instant switch where unsupported.
function toggleTheme(e: Event): void {
  const next = document.documentElement.dataset.theme === "light" ? "dark" : "light";
  const apply = () => {
    config.appearance = next;
    applyAppearance();
    const select = document.querySelector<HTMLSelectElement>("#appearance");
    if (select) select.value = next;
  };

  const btn = e.currentTarget as HTMLElement;
  const rect = btn.getBoundingClientRect();
  const x = rect.left + rect.width / 2;
  const y = rect.top + rect.height / 2;
  const maxRadius = Math.hypot(
    Math.max(x, window.innerWidth - x),
    Math.max(y, window.innerHeight - y),
  );

  const doc = document as Document & { startViewTransition?: (cb: () => void) => { ready: Promise<void> } };
  if (!reduceMotion() && doc.startViewTransition) {
    const transition = doc.startViewTransition(apply);
    transition.ready
      .then(() => {
        document.documentElement.animate(
          [
            { clipPath: `circle(0px at ${x}px ${y}px)` },
            { clipPath: `circle(${maxRadius}px at ${x}px ${y}px)` },
          ],
          {
            duration: 500,
            easing: "cubic-bezier(0.4, 0, 0.2, 1)",
            pseudoElement: "::view-transition-new(root)",
          },
        );
      })
      .catch(() => {});
  } else {
    apply();
  }
  void patchConfig({ appearance: next });
}

systemLight.addEventListener("change", () => {
  if (config.appearance === "system") applyAppearance();
});

// ---------------------------------------------------------------------------
// Customize screen
// ---------------------------------------------------------------------------

function isStarrable(s: Snapshot | undefined, key: string): boolean {
  return s?.metrics.some((m) => m.label === key && m.kind === "progress") ?? false;
}

// Providers start collapsed in Customize; only what you're editing unfolds.
// Session-only — collapsing again on reopen keeps the list scannable.
const custExpanded = new Set<string>();

function renderProviderAccountSettings(family: string): string {
  const keyPlaceholders: Record<string, string> = {
    openrouter: "sk-or-v1-…", zai: t("settings.keyPlaceholder"), commandcode: t("settings.keyPlaceholder"), stepfun: t("settings.keyPlaceholder"),
    minimax: t("settings.keyPhMinimax"), deepseek: "sk-…", kimi: t("settings.keyPhKimi"),
    moonshot: t("settings.keyPhMoonshot"), elevenlabs: t("settings.keyPlaceholder"),
    codebuff: t("settings.keyPhCodebuff"), kilo: t("settings.keyPhKilo"),
    aihubmix: t("settings.keyPhAihubmix"), qwen: t("settings.keyPhQwen"),
  };
  const key = keyPlaceholders[family];
  const keyRow = key === undefined ? "" : `<p class="settings-note">${escapeHtml(t("settings.apiKeysNote"))}</p><div class="key-row"><label for="key-${family}">${escapeHtml(t("settings.apiKeys"))}</label><input id="key-${family}" data-provider-draft="key-${family}" type="password" autocomplete="off" placeholder="${escapeHtml(key)}" /><button type="button" data-save="${family}">${escapeHtml(t("settings.save"))}</button></div>`;
  const tier = family !== "stepfun" ? "" : `<div class="setting-row"><label for="stepfun-plan">${escapeHtml(t("settings.stepfunPlan"))}</label><select id="stepfun-plan"><option value=""${config.stepfunPlanCredits == null ? " selected" : ""}>${escapeHtml(t("settings.stepfunPlanUnset"))}</option>${[[400, "Flash Mini · 400M"], [1600, "Flash Plus · 1,600M"], [8000, "Flash Pro · 8,000M"], [40000, "Flash Max · 40,000M"]].map(([value, label]) => `<option value="${value}"${config.stepfunPlanCredits === value ? " selected" : ""}>${label}</option>`).join("")}</select></div><p class="settings-note">${escapeHtml(t("settings.stepfunPlanHint"))}</p>`;
  const experimental = family === "commandcode"
    ? `<p class="settings-note">${escapeHtml(t("settings.commandcodeExperimental"))}</p><p class="settings-note">${escapeHtml(t("settings.commandcodeKeyHelp"))}</p>`
    : "";
  return experimental + keyRow + tier;
}

function renderCustomize(): string {
  const policy = config.accessPolicy;
  const known = ALL_PROVIDERS.map(([id]) => id);
  const accountOrder = [...new Set([...(config.layout?.providerOrder ?? []), ...known, ...Object.keys(policy.accountBindings)])];
  const families = [...new Set(accountOrder.map(providerFamily))].filter((id) => known.includes(id));
  const blocks = families.map((family) => {
    const name = ALL_PROVIDERS.find(([id]) => id === family)![1];
    const familyOn = policy.enabledFamilies.includes(family);
    const accounts = [family, ...accountOrder.filter((id) => id !== family && policy.accountBindings[id]?.family === family)];
    const count = accounts.filter(accountAuthorized).length;
    const status = t(!familyOn ? "platform.off" : count === 0 ? "platform.noAccounts" : "platform.authorized", { n: count });
    const accountRows = accounts.map((id) => {
      const binding = policy.accountBindings[id];
      const label = binding?.name ?? t("platform.defaultAccount");
      const snapshot = lastSnapshots.find((item) => item.id === id);
      const L = liveProviderLayout(id);
      const row = (key: string) => {
        const starrable = isStarrable(snapshot, key);
        return `<div class="cust-row" draggable="true" data-cust-row="${escapeHtml(id)}|${escapeHtml(key)}"><span class="grip" title="${escapeHtml(t("customize.dragRows"))}">⠿</span><label class="toggle mini" title="${escapeHtml(t("platform.displayOnly"))}"><input type="checkbox" aria-label="${escapeHtml(t("platform.showMetric", { name: displayMetricLabel(key) }))}" data-visible="${escapeHtml(id)}|${escapeHtml(key)}"${L.hidden.includes(key) ? "" : " checked"} /></label><span class="cust-label">${escapeHtml(displayMetricLabel(key))}</span>${starrable ? `<button class="star${L.starred.includes(key) ? " on" : ""}" data-star="${escapeHtml(id)}|${escapeHtml(key)}" title="${escapeHtml(t("customize.star"))}">★</button>` : ""}</div>`;
      };
      const always = L.metricOrder.filter((key) => !L.onDemand.includes(key));
      const onDemand = L.metricOrder.filter((key) => L.onDemand.includes(key));
      const rows = L.metricOrder.length ? `${always.map(row).join("")}<div class="cust-divider" data-divider="${escapeHtml(id)}">${escapeHtml(t("customize.onDemand"))}</div>${onDemand.map(row).join("")}` : `<p class="placeholder">${escapeHtml(t("customize.noData"))}</p>`;
      const active = accountAuthorized(id);
      const accountStatus = !active ? t("platform.off")
        : snapshot?.status === "manual" ? t(isManualQuotaProvider(id) ? "platform.accountManual" : "card.autoPending")
        : isManualQuotaProvider(id) && snapshot?.status === "ok" && snapshot.stale ? t("card.manualSaved")
        : snapshot?.status === "ok" ? t("platform.accountReady")
        : snapshot ? t("platform.accountUnavailable") : t("platform.accountPending");
      return `<section class="platform-account"${binding ? ` data-cust-provider="${escapeHtml(id)}" draggable="true"` : ""}>
        <div class="setting-row platform-account-head">${binding ? `<span class="grip" title="${escapeHtml(t("customize.dragProviders"))}">⠿</span>` : ""}<strong>${escapeHtml(label)}</strong><span class="platform-status">${escapeHtml(accountStatus)}</span></div>
        ${binding ? `<p class="settings-note path">${escapeHtml(binding.directory)}</p>` : ""}
        <label class="toggle platform-account-toggle"><input type="checkbox" data-access-account="${escapeHtml(id)}"${active ? " checked" : ""}${familyOn ? "" : " disabled"} /> ${escapeHtml(t("settings.providerAllowDefault"))}</label>
        ${renderProviderMode(providerModeCatalog.state.entries.find((entry) => entry.id === id), policy.regions[id], t, escapeHtml)}
        ${id === family ? renderProviderAccountSettings(family) : ""}
        <div class="setting-row platform-layout-head"><strong>${escapeHtml(t("platform.metrics"))}</strong><button type="button" class="mini-btn" data-reset="${escapeHtml(id)}" title="${escapeHtml(t("customize.resetLayoutTip"))}">${escapeHtml(t("customize.resetLayout"))}</button></div><p class="settings-note">${escapeHtml(t("platform.displayOnly"))}</p><div class="cust-rows">${rows}</div>
      </section>`;
    }).join("");
    const directory = ["claude", "codex", "opencode"].includes(family) ? `<p class="settings-note">${escapeHtml(t("settings.providerDirectoryHelp"))}</p><form data-access-discover="${family}" class="key-row"><input data-access-directory="${family}" data-provider-draft="directory-${family}" aria-label="${escapeHtml(t("settings.providerDirectory", { name }))}" placeholder="${escapeHtml(t("settings.providerDirectoryPlaceholder"))}"${familyOn ? "" : " disabled"} required /><button type="submit"${familyOn ? "" : " disabled"}>${escapeHtml(t("settings.providerIdentify"))}</button></form>` : "";
    const open = custExpanded.has(family);
    return `<article class="provider customize-block${familyOn ? "" : " muted"}${open ? " open" : ""}" data-cust-provider="${family}" draggable="true"><div class="provider-head"><span class="grip" title="${escapeHtml(t("customize.dragProviders"))}">⠿</span><button type="button" class="cust-expand" data-cust-expand="${family}" aria-expanded="${open}" aria-controls="platform-${family}" title="${escapeHtml(t(open ? "customize.collapse" : "customize.expand"))}"><span class="provider-name">${escapeHtml(name)}</span>${experimentalProviderBadge(family)}<span class="chev">⌄</span></button><span class="platform-status" role="status">${escapeHtml(status)}</span><label class="toggle mini platform-query" title="${escapeHtml(t("platform.queryHelp"))}"><input type="checkbox" data-access-family="${family}"${familyOn ? " checked" : ""} /> ${escapeHtml(t("platform.query"))}</label></div><div id="platform-${family}" class="acc-body"${open ? "" : " inert"}><div class="acc-inner"><p class="settings-note">${escapeHtml(t("platform.queryHelp"))}</p>${renderProviderAutoRefresh(family)}${accountRows}${directory}</div></div></article>`;
  }).join("");
  const starCount = Object.keys(config.layout?.providers ?? {}).reduce((n, id) => n + liveProviderLayout(id).starred.length, 0);
  return `<div class="customize-bar glass-bar"><button class="dock-btn" data-customize-close>${escapeHtml(t("customize.done"))}</button><span class="panel-title">${escapeHtml(t("platform.title"))}</span><button class="dock-btn danger" data-reset-all title="${escapeHtml(t("customize.resetAllTip"))}">${escapeHtml(t("customize.resetAll"))}</button></div><p class="settings-note">${escapeHtml(t("settings.providerAccessHelp"))}</p><p class="settings-note">${escapeHtml(t("customize.starred", { n: starCount }))}</p>${renderProviderModeCatalogStatus(providerModeCatalog.state, t, escapeHtml)}${blocks}`;
}

// ---------------------------------------------------------------------------
// Render root
// ---------------------------------------------------------------------------

function renderWelcome(): string {
  if (config.welcomeDismissed || !lastSnapshots.length) return "";
  return `
    <article class="provider welcome-card">
      <div class="provider-head">
        <span class="provider-name">${escapeHtml(t("welcome.title"))}</span>
        <span class="spacer"></span>
        <button class="share-btn welcome-close" data-welcome-close title="${escapeHtml(t("welcome.dismiss"))}">✕</button>
      </div>
      <p class="placeholder" style="margin:2px 0 8px">
        ${escapeHtml(t("welcome.body"))}
      </p>
      <button class="mini-btn" data-welcome-customize>${escapeHtml(t("welcome.open"))}</button>
    </article>`;
}

// ---------------------------------------------------------------------------
// In-card row dragging — pointer-based, wired onto #providers at init.
// Cards themselves keep their HTML5 grip drag; rows never touch it.
// ---------------------------------------------------------------------------

interface RowDrag {
  id: string;
  card: HTMLElement;
  row: HTMLElement;
  pointerId: number;
  startY: number;
  offsetY: number;
  lifted: boolean;
  fromDemand: boolean;
  placeholder: HTMLElement | null;
  originParent: Node | null;
  originNext: Node | null;
}

let rowDrag: RowDrag | null = null;
// A renderAll arriving mid-gesture is queued, not run — the lifted row
// floats in document.body and a re-render would orphan the gesture.
let rowRenderDeferred = false;

/// Lifts the row out of the card: a placeholder holds its slot while the
/// row follows the pointer as a fixed element in document.body.
function liftRowDrag(d: RowDrag): void {
  const row = d.row;
  const rect = row.getBoundingClientRect();
  d.lifted = true;
  d.originParent = row.parentNode;
  d.originNext = row.nextSibling;
  const ph = document.createElement("div");
  ph.className = "row-placeholder";
  ph.style.height = `${rect.height}px`;
  row.parentNode!.insertBefore(ph, row);
  d.placeholder = ph;
  row.classList.add("row-lifted");
  row.style.width = `${rect.width}px`;
  row.style.left = `${rect.left}px`;
  row.style.top = `${rect.top}px`;
  document.body.appendChild(row);
  try {
    row.setPointerCapture(d.pointerId);
  } catch {
    // The pointer may already be gone; pointerup still settles the drag.
  }
  document.body.classList.add("row-dragging");
  // The pre-lift movement can leave text selected under the pointer.
  window.getSelection()?.removeAllRanges();
  // Hover chrome must not float over a dragged row.
  document.querySelector<HTMLElement>("#model-tip")!.hidden = true;
  resetsPopover.dismiss();
}

/// FLIP pass for the live reorder: measure rows, move the placeholder,
/// then translate each shifted row from its old spot back to zero.
/// Skipped under reduce-anim.
function slideRows(card: HTMLElement, mutate: () => void): void {
  const rows = Array.from(card.querySelectorAll<HTMLElement>("[data-row]"));
  if (document.body.classList.contains("reduce-anim")) {
    mutate();
    return;
  }
  const tops = rows.map((r) => r.getBoundingClientRect().top);
  mutate();
  rows.forEach((r, i) => {
    const dy = tops[i] - r.getBoundingClientRect().top;
    if (!dy) return;
    r.style.transition = "none";
    r.style.transform = `translateY(${dy}px)`;
    requestAnimationFrame(() => {
      r.style.transition = "transform .18s ease";
      r.style.transform = "";
      r.addEventListener("transitionend", () => (r.style.transition = ""), { once: true });
    });
  });
}

/// Follows the pointer: the row tracks vertically while the placeholder
/// live-sorts among the card's rows by midpoint. The expanded .on-demand
/// container is a drop zone of its own (landing there makes the row
/// on-demand; leaving it makes the row always-visible).
function moveRowDrag(d: RowDrag, clientY: number): void {
  d.row.style.top = `${clientY - d.offsetY}px`;
  const panel = d.card.querySelector<HTMLElement>(".card-panel")!;
  const onDemand = panel.querySelector<HTMLElement>(":scope > .on-demand");
  // Never tuck the card's last always-visible row: an empty card would be
  // undone by ensureLayout promoting everything back anyway.
  const canTuck = panel.querySelector(":scope > [data-row]") !== null;
  const zone = onDemand && canTuck && clientY >= onDemand.getBoundingClientRect().top ? onDemand : panel;
  const ph = d.placeholder!;
  const rows = Array.from(zone.querySelectorAll<HTMLElement>(":scope > [data-row]"));
  let before: Element | null = null;
  for (const sib of rows) {
    const r = sib.getBoundingClientRect();
    if (clientY < r.top + r.height / 2) {
      before = sib;
      break;
    }
  }
  if (!before) {
    // Past the last row: land at the run's end (before the quick-links /
    // caret chrome in the always zone; the container end in .on-demand).
    const last = rows[rows.length - 1];
    before = last ? last.nextElementSibling : zone.firstElementChild;
  }
  if (before === ph || ph.nextElementSibling === before) return;
  slideRows(d.card, () => zone.insertBefore(ph, before));
}

/// Slot-fills the card's new DOM order back into the stored layout: only
/// slots whose key was rendered get dealt a new key, so hidden or
/// not-currently-rendered metrics keep their positions. On-demand
/// membership changes only for the dragged row, and only when it crossed
/// zones — sibling membership is left untouched (sub2ApiLiveLayout's
/// projection can render stored-tucked rows above the caret).
function persistRowDrag(d: RowDrag): void {
  ensureLayout();
  // The real stored layout — not sub2ApiLiveLayout's projection, which is
  // what the card renders from but drops keys that aren't live.
  const L = providerLayout(d.id);
  const panel = d.card.querySelector<HTMLElement>(".card-panel")!;
  const keys: string[] = [];
  panel.querySelectorAll<HTMLElement>(":scope > [data-row]").forEach((r) => {
    keys.push(r.dataset.row!);
  });
  panel
    .querySelectorAll<HTMLElement>(":scope > .on-demand > [data-row]")
    .forEach((r) => {
      keys.push(r.dataset.row!);
    });
  const keySet = new Set(keys);
  const queue = [...keys];
  L.metricOrder = L.metricOrder.map((k) => (keySet.has(k) ? queue.shift() ?? k : k));
  // The row is already back in the placeholder's slot, so its zone is
  // readable from the DOM. Only crossing zones flips membership.
  const key = d.row.dataset.row!;
  const nowDemand = !!d.row.parentElement?.classList.contains("on-demand");
  if (nowDemand !== d.fromDemand) {
    L.onDemand = nowDemand
      ? [...L.onDemand, key]
      : L.onDemand.filter((k) => k !== key);
  }
  saveLayout(true);
  renderAll(); // also re-renders the Customize drawer when it is open
}

/// Settles the gesture: commit drops the row where its placeholder sits
/// and persists; cancel restores the row to its pre-lift position.
function finishRowDrag(d: RowDrag, commit: boolean): void {
  rowDrag = null;
  const deferred = rowRenderDeferred;
  rowRenderDeferred = false;
  if (!d.lifted) {
    if (deferred) renderAll();
    return;
  }
  d.row.classList.remove("row-lifted");
  d.row.removeAttribute("style");
  document.body.classList.remove("row-dragging");
  if (commit && d.placeholder) {
    d.placeholder.replaceWith(d.row);
    persistRowDrag(d);
  } else {
    d.placeholder?.remove();
    d.originParent?.insertBefore(d.row, d.originNext);
    if (deferred) renderAll();
  }
  // The release that ends a lifted drag must not also fire the click
  // handlers under the pointer (data-flip, caret, spend rows, ...). The
  // re-render above can destroy the press targets so the click never
  // dispatches — disarm the swallower on the next tick rather than let
  // it linger and eat a later, real click.
  const swallow = (e: MouseEvent): void => {
    e.stopPropagation();
    e.preventDefault();
  };
  document.addEventListener("click", swallow, { capture: true, once: true });
  setTimeout(() => document.removeEventListener("click", swallow, { capture: true }), 0);
}

function renderAll(): void {
  if (rowDrag) {
    rowRenderDeferred = true;
    return;
  }
  const el = document.querySelector("#providers")!;
  const restoreFocus = preserveSettingsFocus(el);
  const scroll = el.scrollTop;
  el.innerHTML =
    (config.accessPolicy.enabledAccounts.length ? "" : `<p class="placeholder">${escapeHtml(t("card.accountsOff"))}</p>`) + renderWelcome() + renderTotalSpend() + renderLocalSpendCards() + orderedSnapshots().map(renderCard).join("");
  el.scrollTop = scroll;
  restoreFocus();
  resetsPopover.onRender();
  if (customizeOpen) renderDrawerBody();
  rebuildTrail();
}

function renderDrawerBody(): void {
  renderProviderAccess();
}

/// Customize lives in a drawer that slides in from the left edge.
function setDrawer(open: boolean): void {
  customizeOpen = open;
  if (open) {
    renderDrawerBody();
    // Local JSON list — cheap, and required if Customize opens before Settings.
  }
  document.body.classList.toggle("drawer-open", open);
  document.querySelector("#customize-btn")?.classList.toggle("active", open);
}

// ---------------------------------------------------------------------------
// Navigation trail: a slim rail of ticks — one per card — that shows where
// you are in the scroll and jumps to a card on click.
// ---------------------------------------------------------------------------

function trailCards(): HTMLElement[] {
  return Array.from(document.querySelectorAll<HTMLElement>("#providers > article"));
}

function rebuildTrail(): void {
  const trail = document.querySelector<HTMLElement>("#trail")!;
  const cards = trailCards();
  if (cards.length < 2) {
    trail.innerHTML = "";
    trail.hidden = true;
    return;
  }
  trail.hidden = false;
  trail.innerHTML = cards
    .map((card, i) => {
      const name = card.querySelector(".provider-name")?.textContent ?? `Card ${i + 1}`;
      return `<button class="trail-tick" data-trail="${i}" title="${escapeHtml(name)}"></button>`;
    })
    .join("");
  // Minimap feel: tick width follows the card's height, like Codex's rail.
  const ticks = trail.querySelectorAll<HTMLElement>(".trail-tick");
  ticks.forEach((tick, i) => {
    const h = cards[i]?.offsetHeight ?? 80;
    tick.style.width = `${Math.max(7, Math.min(16, Math.round(5 + h / 45)))}px`;
  });
  updateTrailActive();
}

/// Codex-style magnetic rail: ticks near the cursor stretch and brighten
/// with a smooth falloff; everything settles back when the mouse leaves.
function setupTrailFisheye(): void {
  const sidebar = document.querySelector<HTMLElement>(".sidebar")!;
  let raf = 0;

  const reset = () => {
    cancelAnimationFrame(raf);
    document.querySelectorAll<HTMLElement>("#trail .trail-tick").forEach((t) => {
      t.style.transform = "";
      t.style.background = "";
    });
  };

  sidebar.addEventListener("mousemove", (e) => {
    const y = e.clientY;
    cancelAnimationFrame(raf);
    raf = requestAnimationFrame(() => {
      document.querySelectorAll<HTMLElement>("#trail .trail-tick").forEach((tick) => {
        const r = tick.getBoundingClientRect();
        const d = Math.abs(y - (r.top + r.height / 2));
        const g = Math.exp(-(d * d) / (2 * 26 * 26)); // gaussian falloff, σ≈26px
        const active = tick.classList.contains("active");
        tick.style.transform = `scaleX(${(1 + 0.9 * g).toFixed(3)})`;
        const mix = Math.round(Math.max(g * 85, active ? 100 : 12));
        tick.style.background = `color-mix(in srgb, var(--foreground) ${mix}%, var(--border))`;
      });
    });
  });
  sidebar.addEventListener("mouseleave", reset);
}

function updateTrailActive(): void {
  const providersEl = document.querySelector<HTMLElement>("#providers")!;
  const cards = trailCards();
  if (!cards.length) return;
  const anchor = providersEl.scrollTop + 70;
  let active = 0;
  for (let i = 0; i < cards.length; i++) {
    if (cards[i].offsetTop <= anchor) active = i;
  }
  // Bottom of the list: light up the last tick even if a tall card above
  // still owns the anchor line.
  if (providersEl.scrollTop + providersEl.clientHeight >= providersEl.scrollHeight - 4) {
    active = cards.length - 1;
  }
  document.querySelectorAll<HTMLElement>("#trail .trail-tick").forEach((tick, i) => {
    tick.classList.toggle("active", i === active);
  });
}

// ---------------------------------------------------------------------------
// Spend row model tooltip
// ---------------------------------------------------------------------------

/// Tooltip for one Usage Trend bar: date, tokens used, share of 30 days.
function showTrendTip(el: HTMLElement): void {
  const tip = document.querySelector<HTMLElement>("#model-tip")!;
  const [id, idxStr] = (el.dataset.trend ?? "").split("|");
  const spend = lastSpend.find((row) => row.id === id);
  const i = Number(idxStr);
  if (!spend || Number.isNaN(i)) return;

  const tokens = spend.trend[i] ?? 0;
  const total = spend.trend.reduce((a, b) => a + b, 0);
  const share = total > 0 ? (tokens / total) * 100 : 0;
  const date = new Date(Date.now() - (29 - i) * 86_400_000).toLocaleDateString(localeTag(), {
    weekday: "short",
    month: "short",
    day: "numeric",
  });
  tip.innerHTML = `
    <div class="tip-line"><span class="tip-name">${escapeHtml(date)}</span><span>${
      tokens > 0 ? escapeHtml(t("card.tokens", { n: fmtTokens(tokens) })) : escapeHtml(t("spend.noUsage"))
    }</span></div>
    ${tokens > 0 ? `<div class="tip-line detail"><span>${escapeHtml(t("spend.of30", { n: share < 1 ? "<1" : share.toFixed(0) }))}</span></div>` : ""}`;

  const rect = el.getBoundingClientRect();
  tip.hidden = false;
  const top = Math.min(rect.bottom + 6, window.innerHeight - tip.offsetHeight - 8);
  tip.style.top = `${Math.max(4, top)}px`;
  tip.style.left = `${Math.max(8, Math.min(rect.left - 50, window.innerWidth - tip.offsetWidth - 8))}px`;
}

function showModelTip(row: HTMLElement): void {
  const tip = document.querySelector<HTMLElement>("#model-tip")!;
  const [id, key] = (row.dataset.spend ?? "").split("|");
  const spend = lastSpend.find((sp) => sp.id === id);
  const w = spend?.[key as SpendTab];
  if (!w) return;

  if (!w.models.length) {
    tip.innerHTML = `<p class="placeholder">${escapeHtml(t("spend.noModelData"))}</p>`;
  } else {
    tip.innerHTML = w.models
      .map((m) => {
        const share = w.cost > 0 ? (m.cost / w.cost) * 100 : 0;
        const unknown = m.unpriced;
        const modelCost = estimatedCostText(fmtMoney(m.cost), m.cost, unknown ? 1 : 0, t);
        return `
          <div class="tip-model">
            <div class="tip-line"><span class="tip-name">${escapeHtml(m.model)}</span><span>${escapeHtml(modelCost)}</span></div>
            <div class="tip-line detail"><span>${unknown ? escapeHtml(t("pricing.unknownPrice")) : `${share.toFixed(0)}%`}</span><span>${escapeHtml(t("card.tokens", { n: fmtTokens(m.tokens) }))}</span></div>
            <div class="tip-bar"><div style="width:${Math.max(2, share)}%"></div></div>
          </div>`;
      })
      .join("");
  }

  const rect = row.getBoundingClientRect();
  tip.hidden = false;
  const top = Math.min(rect.bottom + 4, window.innerHeight - tip.offsetHeight - 8);
  tip.style.top = `${Math.max(4, top)}px`;
  tip.style.left = `${Math.max(8, Math.min(rect.left + 20, window.innerWidth - tip.offsetWidth - 8))}px`;
}

// ---------------------------------------------------------------------------
// Weekly capacity tooltip
// ---------------------------------------------------------------------------

/// One quota cycle in the "Weekly capacity" row's detail JSON. History
/// entries carry `peak_pct` instead of `used_pct`.
interface CapacityCycle {
  start_ms: number;
  end_ms: number;
  used_pct?: number;
  peak_pct?: number;
  tokens: number;
  cost: number;
  est_tokens?: number;
  est_cost?: number;
  status: "active" | "observed" | "incomplete";
  /// Null on a finalizing cycle: it reached 100% and rolled over, but
  /// no post-hit scan has confirmed its totals yet.
  observed_at_ms?: number | null;
}

interface CapacityDetail {
  current?: CapacityCycle;
  history?: CapacityCycle[];
  avg?: { tokens: number; cost: number; n: number };
}

function parseCapacityDetail(detail: string | null): CapacityDetail | null {
  if (!detail) return null;
  try {
    const parsed = JSON.parse(detail);
    return parsed && typeof parsed === "object" ? (parsed as CapacityDetail) : null;
  } catch {
    return null;
  }
}

/// Whole-dollar rounding for estimates — "≈ $218" reads as the guess it
/// is; observed figures keep cents.
function fmtEstMoney(v: number): string {
  if (v >= 1000) return `$${(v / 1000).toFixed(1)}K`;
  if (v >= 100) return `$${v.toFixed(0)}`;
  return `$${v.toFixed(2)}`;
}

/// The row's right-hand value, localized from the detail JSON.
function capacityRowText(d: CapacityDetail): string {
  const cur = d.current;
  if (!cur) return "";
  const known = cur.tokens > 0 || cur.cost > 0;
  if (cur.status === "observed") {
    return t("cap.valueObserved", { cost: fmtMoney(cur.cost), n: fmtTokens(cur.tokens) });
  }
  if (cur.est_cost !== undefined && cur.est_tokens !== undefined) {
    return t("cap.valueEst", { cost: fmtEstMoney(cur.est_cost), n: fmtTokens(cur.est_tokens) });
  }
  if (!known) return t("cap.collecting");
  return t("cap.valueSoFar", { cost: fmtMoney(cur.cost), n: fmtTokens(cur.tokens) });
}

/// "Sep 12" — the date half of a past-week line.
function fmtWeekDay(ms: number): string {
  return new Date(ms).toLocaleDateString(localeTag(), { month: "short", day: "numeric" });
}

/// Hover tooltip on the capacity value chip: current cycle, past weeks,
/// observed average, and the API-equivalent disclaimer. Same surface and
/// typography as the spend rows' per-model breakdown (#model-tip).
function showCapacityTip(el: HTMLElement): void {
  const tip = document.querySelector<HTMLElement>("#model-tip")!;
  const [id] = (el.dataset.cap ?? "").split("|");
  const metric = lastSnapshots
    .find((s) => s.id === id)
    ?.metrics.find((m) => m.label === "Weekly capacity");
  const d = metric ? parseCapacityDetail(metric.detail) : null;
  const cur = d?.current;
  if (!cur) return;

  const known = cur.tokens > 0 || cur.cost > 0;
  const hasEst = cur.est_tokens !== undefined && cur.est_cost !== undefined;
  const lines: string[] = [
    `<div class="tip-line"><span class="tip-name">${escapeHtml(t("cap.thisWeek"))}</span><span>${escapeHtml(t("card.pctUsed", { n: Math.round(cur.used_pct ?? cur.peak_pct ?? 0) }))}</span></div>`,
    `<div class="tip-line"><span class="tip-name">${escapeHtml(t("cap.observedSoFar"))}</span><span>${
      known ? `${escapeHtml(fmtTokens(cur.tokens))} · ${escapeHtml(fmtMoney(cur.cost))}` : "—"
    }</span></div>`,
    `<div class="tip-line"><span class="tip-name">${escapeHtml(t("cap.estAt100"))}</span><span>${
      hasEst ? `~${escapeHtml(fmtTokens(cur.est_tokens!))} · ~${escapeHtml(fmtEstMoney(cur.est_cost!))}` : "—"
    }</span></div>`,
  ];

  const history = (d!.history ?? []).slice(0, 6);
  if (history.length) {
    lines.push(`<div class="tip-line detail cap-gap"><span>${escapeHtml(t("cap.pastWeeks"))}</span></div>`);
    for (const c of history) {
      const status =
        c.status === "observed"
          ? c.observed_at_ms == null
            ? `${t("cap.observed")} · ${t("cap.finalizing")}`
            : t("cap.observed")
          : t("cap.incompletePeak", { n: Math.round(c.peak_pct ?? 0) });
      lines.push(
        `<div class="tip-line detail"><span>${escapeHtml(fmtWeekDay(c.start_ms))} – ${escapeHtml(fmtWeekDay(c.end_ms))} · ${escapeHtml(status)}</span><span>${escapeHtml(fmtTokens(c.tokens))} · ${escapeHtml(fmtMoney(c.cost))}</span></div>`,
      );
    }
  }
  if (d!.avg) {
    lines.push(
      `<div class="tip-line"><span class="tip-name">${escapeHtml(t("cap.avg"))}</span><span>~${escapeHtml(fmtTokens(d!.avg.tokens))} · ~${escapeHtml(fmtEstMoney(d!.avg.cost))} (${d!.avg.n})</span></div>`,
    );
  }
  lines.push(`<div class="tip-foot">${escapeHtml(t("cap.footnote"))}</div>`);
  tip.innerHTML = lines.join("");

  const rect = el.getBoundingClientRect();
  tip.hidden = false;
  const top = Math.min(rect.bottom + 4, window.innerHeight - tip.offsetHeight - 8);
  tip.style.top = `${Math.max(4, top)}px`;
  tip.style.left = `${Math.max(8, Math.min(rect.left + 20, window.innerWidth - tip.offsetWidth - 8))}px`;
}

// ---------------------------------------------------------------------------
// Rate Limit Resets popover
// ---------------------------------------------------------------------------

interface RedeemOutcome {
  outcome: "success" | "nothing_to_reset" | "no_credit";
  message: string;
  windows_reset: number;
}

/// Upstream's HoverPopoverState + RateLimitResetsDetail in one controller:
/// a 400ms dwell on the row's value opens the timeline, a 180ms grace lets
/// the cursor travel into the popover, and the confirm → claim flow pins
/// it so a cursor slip can't tear down a live claim.
const resetsPopover = (() => {
  let providerId = "";
  let label = "";
  let open = false;
  let overInline = false;
  let overDetail = false;
  let pinned = false;
  let showTimer: ReturnType<typeof setTimeout> | null = null;
  let hideTimer: ReturnType<typeof setTimeout> | null = null;
  // Credits claimed this session, keyed by credit id — a claimed node drops
  // out of the timeline immediately instead of waiting for the refresh.
  let claimed = new Set<string>();
  // The node currently in its inline confirm, or being claimed, keyed like
  // `claimed` (id, else `exp:<ms>` — only id'd credits are claimable).
  let confirming: string | null = null;
  let claiming: string | null = null;
  // The node the cursor is over (drives the Use reveal).
  let hovered: string | null = null;
  // The credits the last render drew — nodeHover needs them for the
  // countdown ⇄ Use swap without re-reading the snapshot.
  let visible: ResetCredit[] = [];
  // Per-credit idempotency keys, minted on first confirm and reused on
  // every retry — a retried claim can never double-spend (the server
  // answers already_redeemed, which counts as success).
  const keys = new Map<string, string>();
  let banner: { kind: "success" | "info" | "warn" | "error"; text: string } | null = null;
  // True once a claim reset usage or the server refused with
  // nothing_to_reset: the remaining Use buttons disable until the popover
  // closes — by then real usage may have resumed.
  let nothingToReset = false;

  const pop = () => document.querySelector<HTMLElement>("#resets-pop")!;
  const creditKey = (c: ResetCredit) => c.id ?? `exp:${c.expires_at}`;
  const claimBusy = () => confirming !== null || claiming !== null;

  function liveMetric(): Metric | null {
    return (
      lastSnapshots
        .find((s) => s.id === providerId)
        ?.metrics.find((m) => m.label === label) ?? null
    );
  }

  function findAnchor(): HTMLElement | null {
    const wanted = `${providerId}|${label}`;
    return (
      Array.from(document.querySelectorAll<HTMLElement>("[data-resets]")).find(
        (a) => a.dataset.resets === wanted,
      ) ?? null
    );
  }

  function setHot(hot: boolean): void {
    findAnchor()?.classList.toggle("hot", hot);
  }

  function position(): void {
    const anchor = findAnchor();
    const el = pop();
    if (!anchor) return;
    const rect = anchor.getBoundingClientRect();
    el.hidden = false;
    // Below the value, right edges aligned, clamped inside the viewport.
    const top = Math.min(rect.bottom + 4, window.innerHeight - el.offsetHeight - 8);
    el.style.top = `${Math.max(4, top)}px`;
    el.style.left = `${Math.max(8, Math.min(rect.right - el.offsetWidth, window.innerWidth - el.offsetWidth - 8))}px`;
  }

  function close(): void {
    if (showTimer) clearTimeout(showTimer);
    if (hideTimer) clearTimeout(hideTimer);
    showTimer = hideTimer = null;
    open = false;
    overInline = overDetail = pinned = false;
    hovered = confirming = claiming = null;
    visible = [];
    banner = null;
    nothingToReset = false;
    setHot(false);
    pop().hidden = true;
  }

  function scheduleHide(): void {
    if (hideTimer) clearTimeout(hideTimer);
    hideTimer = setTimeout(() => {
      hideTimer = null;
      if (!overInline && !overDetail && !pinned) close();
    }, 180);
  }

  /// Fresh per-target claim state — `claimed`/`keys` survive a close (a
  /// reopened popover shouldn't re-offer a just-claimed credit or mint a
  /// second idempotency key) but die when the anchor moves to another row.
  function retarget(pid: string, lbl: string): void {
    if (pid === providerId && lbl === label) return;
    providerId = pid;
    label = lbl;
    claimed = new Set();
    keys.clear();
    confirming = claiming = hovered = null;
    visible = [];
    banner = null;
    nothingToReset = false;
  }

  const CLOCK_SVG = `<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="9"/><path d="M12 7v5l3.2 1.8"/></svg>`;

  function bannerHtml(): string {
    if (!banner) return "";
    const glyph = { success: "✓", info: "i", warn: "⚠", error: "✕" }[banner.kind];
    return `<div class="rs-banner ${banner.kind}"><span class="rs-banner-icon">${glyph}</span>${escapeHtml(banner.text)}</div>`;
  }

  function useBtnHtml(c: ResetCredit, key: string): string {
    if (c.id === undefined || hovered !== key || claimBusy()) return "";
    const off = nothingToReset
      ? ` disabled title="${escapeHtml(t("resets.nothingToResetTip"))}"`
      : "";
    return `<button class="rs-use" data-rs-use="${escapeHtml(key)}"${off}>${escapeHtml(t("resets.use"))}</button>`;
  }

  function countdownHtml(c: ResetCredit): string {
    if (c.expires_at === null) return "";
    const remaining = c.expires_at - Date.now();
    // A past-due or ≤5-minute expiry can't print a useful countdown.
    return remaining > 5 * 60_000 ? escapeHtml(fmtDuration(remaining)) : "";
  }

  /// One node's body: resting line, inline confirm card, or the in-flight
  /// spinner row — the numbered dot stays on the rail either way.
  function nodeBodyHtml(c: ResetCredit, key: string): string {
    if (confirming === key) {
      return `<div class="rs-confirm">
        <div class="rs-q">${escapeHtml(t("resets.confirmTitle"))}</div>
        <div class="rs-sub">${escapeHtml(t("resets.confirmBody"))}</div>
        <div class="rs-actions">
          <button class="rs-go" data-rs-go="${escapeHtml(key)}">${escapeHtml(t("resets.reset"))}</button>
          <button class="rs-cancel" data-rs-cancel>${escapeHtml(t("resets.cancel"))}</button>
        </div>
      </div>`;
    }
    if (claiming === key) {
      return `<span class="rs-time detail">${escapeHtml(t("resets.resetting"))}</span><span class="rs-spinner"></span>`;
    }
    const remaining = c.expires_at === null ? null : c.expires_at - Date.now();
    const imminent = remaining !== null && remaining <= 5 * 60_000;
    const time =
      c.expires_at === null
        ? t("resets.expiryUnknown")
        : imminent
          ? t("resets.expiringSoon")
          : fmtExact(c.expires_at);
    return `<span class="rs-time">${escapeHtml(time)}</span><span class="rs-trail">${useBtnHtml(c, key) || countdownHtml(c)}</span>`;
  }

  function nodeHtml(c: ResetCredit, key: string, i: number, last: boolean): string {
    const sev = c.expires_at === null ? "normal" : expirySeverity(c.expires_at - Date.now());
    const cls = [
      "rs-node",
      i === 0 ? "first" : "",
      last ? "last" : "",
      claimBusy() && confirming !== key && claiming !== key ? "dim" : "",
    ]
      .filter(Boolean)
      .join(" ");
    return `<div class="${cls}" data-rs-credit="${escapeHtml(key)}">
      <div class="rs-rail"><span class="rs-dot ${sev}">${i + 1}</span></div>
      <div class="rs-body">${nodeBodyHtml(c, key)}</div>
    </div>`;
  }

  function render(): void {
    const m = liveMetric();
    const el = pop();
    if (!m) {
      close();
      return;
    }
    const all = parseResetCredits(m);
    // Only subtract claimed credits still present in the data — a refresh
    // that already dropped one would double-count the subtraction.
    const claimedNow = all?.filter((c) => claimed.has(creditKey(c))).length ?? 0;
    visible = (all ?? []).filter((c) => !claimed.has(creditKey(c)));
    const count = Math.max(0, (Number(m.value ?? 0) || 0) - claimedNow);

    // A credit awaiting confirmation vanished from the data (background
    // refresh): fold the card and release the pin rather than stranding a
    // pinned popover on a dead node. An in-flight claim owns its state.
    if (confirming && !visible.some((c) => creditKey(c) === confirming)) {
      confirming = null;
      pinned = false;
    }

    // The row itself left the DOM (card collapsed, provider hidden) —
    // nothing to anchor the popover to.
    if (!findAnchor()) {
      close();
      return;
    }

    let html = bannerHtml();
    if (claiming && !visible.some((c) => creditKey(c) === claiming)) {
      // The claim's own forced refresh can drop the in-flight credit a
      // beat before the outcome resolves — keep the spinner row alive so
      // it hands off to the banner instead of blinking out. Only a
      // surviving timeline renders under it; the empty state under a
      // still-running spinner would read as a contradiction.
      html += `<div class="rs-body rs-detached"><span class="rs-time detail">${escapeHtml(t("resets.resetting"))}</span><span class="rs-spinner"></span></div>`;
      html += visible
        .map((c, i) => nodeHtml(c, creditKey(c), i, i === visible.length - 1))
        .join("");
    } else if (visible.length > 0) {
      html += visible
        .map((c, i) => nodeHtml(c, creditKey(c), i, i === visible.length - 1))
        .join("");
    } else if (count > 0) {
      // Credits exist but the expiry list wasn't fetched (usage-body
      // count fallback) — state the count, not "no resets".
      html += `<div class="rs-empty">${CLOCK_SVG}
        <div>${escapeHtml(t("card.nAvailable", { n: count }))}</div>
        <div class="detail">${escapeHtml(t("resets.expiryUnknown"))}</div></div>`;
    } else {
      html += `<div class="rs-empty">${CLOCK_SVG}<div>${escapeHtml(t("resets.none"))}</div></div>`;
    }
    el.innerHTML = html;
    position();
  }

  /// The countdown ⇄ Use swap on node hover re-renders just that node's
  /// trail — rebuilding the whole popover on every mouseover flickers.
  function updateTrail(key: string | null): void {
    if (key === null) return;
    const node = pop().querySelector<HTMLElement>(`[data-rs-credit="${key}"]`);
    const trail = node?.querySelector<HTMLElement>(".rs-trail");
    const credit = visible.find((c) => creditKey(c) === key);
    if (!trail || !credit) return;
    trail.innerHTML = useBtnHtml(credit, key) || countdownHtml(credit);
  }

  function runClaim(key: string): void {
    const credit = visible.find((c) => creditKey(c) === key);
    confirming = null;
    claiming = key;
    render();
    const status = document.querySelector("#status")!;
    status.textContent = t("footer.redeeming");
    // Claude's banked resets redeem through its own endpoint; the claim
    // flow and outcome vocabulary are otherwise identical to Codex's.
    const command =
      providerId === "claude" || providerId.startsWith("claude@")
        ? "claude_redeem_credit"
        : "codex_redeem_credit";
    invoke<RedeemOutcome>(command, {
      creditId: credit?.id ?? key,
      providerId,
      redeemRequestId: keys.get(key),
    })
      .then(async (o) => {
        status.textContent = o.message;
        if (o.outcome === "success") {
          claimed.add(key);
          nothingToReset = true;
          banner = { kind: "success", text: t("resets.claimed") };
          // Await the forced refresh while still pinned: its renderAll
          // re-renders the popover (banner survives) instead of closing it.
          await refresh(true);
        } else if (o.outcome === "nothing_to_reset") {
          // The server spent no credit — the rest would refuse the same
          // way right now, so the Use buttons disable for this session.
          nothingToReset = true;
          banner = { kind: "info", text: t("resets.noNeed") };
        } else {
          claimed.add(key);
          banner = { kind: "warn", text: t("resets.gone") };
          await refresh(true);
        }
      })
      .catch((err) => {
        banner = { kind: "error", text: t("resets.failed") };
        status.textContent = t("footer.redeemFailed", { err: String(err) });
      })
      .finally(() => {
        claiming = null;
        pinned = false;
        if (open) render();
        scheduleHide();
      });
  }

  return {
    /// Cursor entered the row's value chip.
    inlineEnter(target: HTMLElement): void {
      const [pid, lbl] = (target.dataset.resets ?? "").split("|");
      if (!pid || !lbl) return;
      retarget(pid, lbl);
      overInline = true;
      if (hideTimer) {
        clearTimeout(hideTimer);
        hideTimer = null;
      }
      setHot(true);
      if (open) {
        render();
        return;
      }
      if (showTimer) return;
      showTimer = setTimeout(() => {
        showTimer = null;
        if (!overInline) return;
        open = true;
        render();
      }, 400);
    },
    /// Cursor left the row's value chip entirely.
    inlineLeave(): void {
      overInline = false;
      setHot(open);
      scheduleHide();
    },
    detailEnter(): void {
      overDetail = true;
      if (hideTimer) {
        clearTimeout(hideTimer);
        hideTimer = null;
      }
    },
    detailLeave(): void {
      overDetail = false;
      scheduleHide();
    },
    /// Node hover inside the popover: reveal that credit's Use button.
    nodeHover(key: string | null): void {
      if (claimBusy()) return;
      if (key === hovered) return;
      const prev = hovered;
      hovered = key;
      updateTrail(prev);
      updateTrail(hovered);
    },
    /// Click delegation inside the popover.
    click(target: HTMLElement): void {
      const use = target.closest<HTMLElement>("[data-rs-use]");
      if (use) {
        const key = use.dataset.rsUse!;
        if (!keys.has(key)) keys.set(key, crypto.randomUUID());
        banner = null;
        hovered = null;
        confirming = key;
        pinned = true;
        render();
        return;
      }
      if (target.closest("[data-rs-cancel]")) {
        confirming = null;
        pinned = false;
        render();
        scheduleHide();
        return;
      }
      const go = target.closest<HTMLElement>("[data-rs-go]");
      if (go) runClaim(go.dataset.rsGo!);
    },
    /// Every card-list re-render rebuilds the anchor under us — re-read
    /// the fresh metric, re-find the anchor and re-light the chip rather
    /// than closing; render() itself closes when the row or its metric is
    /// gone. A routine refresh must not kill an open timeline or banner.
    onRender(): void {
      if (!open) return;
      render();
      setHot(true);
    },
    onScroll(): void {
      if (open && !pinned) close();
    },
    dismiss(): void {
      close();
    },
  };
})();

// ---------------------------------------------------------------------------
// Refresh + tray strip
// ---------------------------------------------------------------------------

/// Background refreshes must not pay DOM costs nobody can see: while the
/// popover is hidden (99% of the time), rendering is deferred to the next
/// open instead of rebuilding a filter-heavy DOM every refresh interval.
let pendingRender = false;

function renderIfVisible(): void {
  if (document.hidden) {
    pendingRender = true;
    return;
  }
  pendingRender = false;
  renderAll();
  populatePinnedOptions();
}

function hideFoldedMoonshot(snapshots: Snapshot[]): Snapshot[] {
  // The backend leaves independently queried wallet data separate when the
  // Kimi plan is saved-only. Do not hide or retime that newer wallet result.
  if (isManualQuotaProvider("kimi") && !isManualQuotaProvider("moonshot")) return snapshots;
  const kimi = snapshots.find((s) => s.id === "kimi" && s.status === "ok");
  if (!kimi) return snapshots;
  const wallet = (s: Snapshot) =>
    s.metrics.some((m) => ["API", "Credits used", "Balance", "Vouchers", "Cash"].includes(m.label));
  const moon = snapshots.find((s) => s.id === "moonshot");
  if (wallet(kimi) || !moon || moon.metrics.length === 0) {
    return snapshots.filter((s) => s.id !== "moonshot");
  }
  return snapshots;
}

/// First paint from the previous run's snapshots (disk cache): numbers on
/// screen in milliseconds instead of a blank "Refreshing…" while the
/// slowest provider answers — at boot that wait ran 30-40 seconds. Cards
/// arrive marked stale ("Outdated") and the live fetch replaces them.
async function paintCachedSnapshots(): Promise<void> {
  // Only when a saved layout exists: on a true first run there is no cache
  // anyway. Existing cache responses still carry a backend publication revision.
  if (config.layout === null) return;
  try {

    const requestAccessEpoch = accessEpoch;
    const result = await invoke<UsageResult>("cached_usage");
    const cached = hideFoldedMoonshot(result.snapshots).filter((snapshot) => accountAuthorized(snapshot.id));
    if (requestAccessEpoch !== accessEpoch) return;
    // The live fetch may have already landed — never paint over it.
    if (!cached.length || lastSnapshots.length || lastUsageRevision !== null) return;
    lastSnapshots = cached;
    lastUsageRevision = result.revision;
    scheduleResetRefresh();
    ensureLayout();
    renderIfVisible();
    requestTraySync();
  } catch {
    // No cache readable — the live fetch paints, as before.
  }
}

function updateAccountRefreshButtons(): void {
  document.querySelectorAll<HTMLButtonElement>("[data-account-refresh]").forEach(button => {
    const id = button.dataset.accountRefresh!;
    const snapshot = lastSnapshots.find(item => item.id === id);
    const busy = accountRefreshBusy(id);
    button.disabled = busy || !canRefreshAccount(id);
    button.setAttribute("aria-busy", String(busy));
    const label = accountRefreshLabel(id, snapshot?.name ?? id);
    button.title = label;
    button.setAttribute("aria-label", label);
    delete button.dataset.tip;
  });
}

function cancelQueuedAccountRefreshes(): void {
  refreshQueuedAccounts.clear();
  accountRefreshErrors.clear();
  updateAccountRefreshButtons();
}

function setRefreshLock(on: boolean): void {
  refreshing = on;
  const global = on && activeRefreshScope?.kind === "all";
  document.body.classList.toggle("refreshing", global);
  document.querySelector("#refresh")?.setAttribute("aria-busy", String(global));
  updateAccountRefreshButtons();
}

// Accept backend membership and safety corrections, while retaining equal
// cards' DOM, focus and scroll. A scoped quota query never repaints local logs.
function renderScopedUsage(previous: Snapshot[], target: string): void {
  if (document.hidden) { pendingRender = true; return; }
  if (rowDrag) { rowRenderDeferred = true; return; }
  const root = document.querySelector<HTMLElement>("#providers")!;
  const restoreFocus = preserveSettingsFocus(root);
  const scroll = root.scrollTop;
  const visible = orderedSnapshots();
  const byId = new Map(previous.map(snapshot => [snapshot.id, snapshot]));
  const cards = new Map(Array.from(root.querySelectorAll<HTMLElement>(":scope > [data-provider]"))
    .map(card => [card.dataset.provider!, card]));
  for (const [id, card] of cards) if (!visible.some(snapshot => snapshot.id === id)) card.remove();
  let next: ChildNode | null = null;
  for (const snapshot of [...visible].reverse()) {
    let card = cards.get(snapshot.id);
    if (!card || snapshot.id === target || JSON.stringify(byId.get(snapshot.id)) !== JSON.stringify(snapshot)) {
      const holder = document.createElement("div");
      holder.innerHTML = renderCard(snapshot);
      const replacement = holder.firstElementChild as HTMLElement;
      if (card) card.replaceWith(replacement);
      card = replacement;
    }
    if (card.parentElement !== root || card.nextSibling !== next) root.insertBefore(card, next);
    next = card;
  }
  root.scrollTop = scroll;
  restoreFocus();
  resetsPopover.onRender();
  if (customizeOpen) renderDrawerBody();
  rebuildTrail();
  populatePinnedOptions();
}


function completeRefreshAttempt(generation: number): void {
  completedRefreshGeneration = Math.max(completedRefreshGeneration, generation);
  for (let index = refreshAttemptWaiters.length - 1; index >= 0; index -= 1) {
    if (refreshAttemptWaiters[index].generation <= completedRefreshGeneration) {
      refreshAttemptWaiters.splice(index, 1)[0].resolve();
    }
  }
}

async function forceUsageRefreshAttempt(usageOnly = true): Promise<void> {
  const generation = refreshGeneration + 1;
  const completed = new Promise<void>((resolve) => {
    refreshAttemptWaiters.push({ generation, resolve });
  });
  void refresh(true, usageOnly);
  await completed;
}

async function refresh(force = false, usageOnly = false, reason: UsageRefreshReason = "automatic", scope: UsageRefreshScope = { kind: "all" }): Promise<void> {
  // Revalidate timer/reset/settings intent after the latest preference write.
  // Manual clicks remain independent and still use their own authority fence.
  if (reason === "automatic") while (providerAutoRefreshIntents.size > 0) await configSaveQueue;
  const accountId = scope.kind === "account" ? scope.accountId : null;
  if (accountId !== null && (!canRefreshAccount(accountId) || reason !== "userRefresh")) return;
  // Card requests cannot acquire spend scanning through any queue merge.
  if (accountId !== null) usageOnly = true;
  if (reason === "userRefresh" && pendingAccessWrites > 0) {
    // Backend authority can change before its IPC reply arrives. A click
    // during that uncertainty must not acquire the eventual expanded scope.
    document.querySelector("#status")!.textContent = t("footer.refreshWaitForAccess");
    return;
  }
  if (refreshing) {
    if (accountId !== null) {
      if (!accountRefreshBusy(accountId)) refreshQueuedAccounts.set(accountId, accessEpoch);
      updateAccountRefreshButtons();
      return;
    }
    // Remember a forced request instead of dropping it: the in-flight
    // fetch may have started before whatever prompted this one (a saved
    // key, a toggle), so one more pass runs when it finishes.
    if (force) {
      refreshQueuedUsageOnly = refreshQueued ? refreshQueuedUsageOnly && usageOnly : usageOnly;
      if (reason === "userRefresh") refreshQueuedManualEpoch = accessEpoch;
      refreshQueued = true;
      // The save message (or a stale "Updated") would otherwise sit on
      // the footer until the in-flight fetch ends, and Refresh looks dead.
      document.querySelector("#status")!.textContent = t("footer.refreshing");
    }
    return;
  }
  if (!force && Date.now() - lastFetch < STALE_MS) return;
  activeRefreshScope = scope;
  if (accountId !== null) {
    accountRefreshErrors.delete(accountId);
    document.querySelectorAll<HTMLElement>("[data-provider]").forEach(card => {
      if (card.dataset.provider === accountId) card.querySelector(".account-refresh-error")?.remove();
    });
  }
  const previousSnapshots = lastSnapshots;
  const restoreAccountFocus = accountId !== null && document.activeElement instanceof HTMLElement &&
    document.activeElement.dataset.accountRefresh === accountId;
  setRefreshLock(true);
  const myGen = ++refreshGeneration;
  const status = document.querySelector("#status")!;
  if (accountId === null) status.textContent = t("footer.refreshing");
  // The spend scan re-reads every session log on a cold start and can take
  // tens of seconds — it must never hold up the usage cards' first paint,
  // or the Refresh button (the lock used to stay on until spend finished).
  const requestAccessEpoch = accessEpoch;
  const requestPriceEpoch = priceEpoch;
  const spendPromise = usageOnly || (reason === "automatic" && providerAutoRefreshUncertain)
    ? Promise.resolve<SpendResult | null>(null)
    : invoke<SpendResult>("fetch_spend", { reason }).catch(() => null);
  try {
    const result = reason === "automatic" && providerAutoRefreshUncertain
      ? await invoke<UsageResult>("cached_usage")
      : await invoke<UsageResult>("fetch_usage", { disabled: [...config.disabled], reason, ...(scope.kind === "account" ? { scope } : {}) });
    let snapshots = result.snapshots;
    if (requestAccessEpoch !== accessEpoch || pendingAccessWrites > 0 || (lastUsageRevision !== null && result.revision < lastUsageRevision)) return;
    snapshots = hideFoldedMoonshot(snapshots).filter((snapshot) => accountAuthorized(snapshot.id));
    const firstData = lastSnapshots.length === 0;
    if (accountId === null) lastFetch = Date.now();
    lastSnapshots = snapshots;
    if (accountId === null) accountRefreshErrors.clear();
    lastUsageRevision = result.revision;
    scheduleResetRefresh();
    ensureLayout();
    if (!lastLayoutSnapshot && config.layout) {
      lastLayoutSnapshot = JSON.stringify(config.layout);
    }
    if (accountId === null) renderIfVisible();
    if (firstData && !customizeOpen && !document.hidden) playReveal();
    requestTraySync();
    const time = new Date().toLocaleTimeString(localeTag(), { hour: "2-digit", minute: "2-digit" });
    if (accountId === null) status.textContent = configSaveError
      ? t(providerAutoRefreshUncertain ? "footer.autoRefreshUnknown" : "footer.configSaveFailed", { err: configSaveError })
      : t("footer.updated", { time });
  } catch (err) {
    if (accountId !== null) {
      if (requestAccessEpoch === accessEpoch && pendingAccessWrites === 0 && canRefreshAccount(accountId)) accountRefreshErrors.set(accountId, String(err));
    } else status.textContent = configSaveError
      ? t(providerAutoRefreshUncertain ? "footer.autoRefreshUnknown" : "footer.configSaveFailed", { err: configSaveError })
      : t("footer.refreshFailed", { err: String(err) });
  } finally {
    activeRefreshScope = null;
    setRefreshLock(false);
    if (accountId !== null) {
      renderScopedUsage(previousSnapshots, accountId);
      // A periodic repaint can detach the focused disabled button. Restore
      // that action only if the user has not focused another control since.
      if (restoreAccountFocus && document.activeElement === document.body) {
        Array.from(document.querySelectorAll<HTMLButtonElement>("[data-account-refresh]"))
          .find(button => button.dataset.accountRefresh === accountId && !button.disabled)?.focus({ preventScroll: true });
      }
    }
    completeRefreshAttempt(myGen);
    if (refreshQueued) {
      refreshQueued = false;
      const queuedUsageOnly = refreshQueuedUsageOnly;
      const queuedReason = refreshQueuedManualEpoch === accessEpoch ? "userRefresh" : "automatic";
      refreshQueuedUsageOnly = true;
      refreshQueuedManualEpoch = null;
      void refresh(true, queuedUsageOnly, queuedReason);
    } else {
      for (const [id, epoch] of refreshQueuedAccounts) {
        refreshQueuedAccounts.delete(id);
        if (epoch !== accessEpoch || pendingAccessWrites > 0 || !canRefreshAccount(id)) continue;
        void refresh(true, true, "userRefresh", { kind: "account", accountId: id });
        break;
      }
      updateAccountRefreshButtons();
    }
  }
  const spend = await spendPromise;
  if (usageOnly || requestAccessEpoch !== accessEpoch || !acceptPriceEpoch(requestPriceEpoch, priceEpoch)) return;
  spendLoaded = true;
  if (spend && acceptSpendBatch(requestAccessEpoch, accessEpoch, myGen, lastAppliedSpendGen)) {
    const updated = spend.rows.filter((row) => spendVisible(row) && (row.sources.length === 0 || row.scan_revision === spend.revision));
    const savedCursor = spend.preserveCursor === true && spend.revision === lastUsageRevision
      && spend.revision === lastCursorSpendRevision && lastSnapshots.some(snapshot => snapshot.id === "cursor")
      && !updated.some(row => row.id === "cursor" && row.sources.length === 0)
      ? lastSpend.filter(row => row.id === "cursor" && row.sources.length === 0 && spendVisible(row)) : [];
    lastSpend = [...updated, ...savedCursor];
    lastCursorSpendRevision = lastSpend.some(row => row.id === "cursor" && row.sources.length === 0) ? spend.revision : null;
    lastAppliedSpendGen = myGen;
  }
  if (lastSnapshots.length) ensureLayout();
  if (!customizeOpen) renderIfVisible();
}

function scheduleAutoRefresh(): void {
  if (refreshTimer !== undefined) window.clearInterval(refreshTimer);
  const minutes = Math.max(1, config.refreshMinutes || 5);
  refreshTimer = window.setInterval(() => void refresh(), minutes * 60 * 1000);
}

// One-shot refresh ~30 s after the soonest upcoming long-window reset,
// so the reset toast lands within ~a minute of the rollover instead of
// waiting for the auto-refresh interval. A provider that hasn't rolled
// the window yet gets ONE retry 90 s later, then we stop guessing.
let resetRefreshTimer: number | undefined;
let resetRetryFor: number | null = null;
const RESET_LONG_WINDOW_MS = 6 * 24 * 3_600_000;

/// Long-window verdicts for metrics that report no period, keyed by
/// "<snapshot id>:<label>". Decided once per observed resets_at (a reset
/// ≥6 days out, or a ≥6-day jump from the previous reset) and kept until
/// resets_at changes — never re-derived from the shrinking countdown.
/// One entry per metric keeps the map tiny; no eviction needed.
const inferredLongWindow = new Map<string, { resetsAt: number; long: boolean }>();

function scheduleResetRefresh(): void {
  if (resetRefreshTimer !== undefined) {
    window.clearTimeout(resetRefreshTimer);
    resetRefreshTimer = undefined;
  }
  if (!config.notifyReset) return;
  const now = Date.now();
  let soonest: number | null = null;
  let providerLagging = false;
  for (const s of lastSnapshots) {
    if (s.status !== "ok" || s.stale || isManualQuotaProvider(s.id)) continue;
    for (const m of s.metrics) {
      if (m.kind !== "progress" || m.resets_at === null) continue;
      let long: boolean;
      if (m.period_ms !== null) {
        long = m.period_ms >= RESET_LONG_WINDOW_MS;
      } else {
        // No declared period: decide once per resets_at and keep the
        // verdict — re-deriving it from the countdown would flip a real
        // weekly to short as the reset approaches.
        const key = `${s.id}:${m.label}`;
        const prev = inferredLongWindow.get(key);
        if (prev && prev.resetsAt === m.resets_at) {
          long = prev.long;
        } else {
          long = m.resets_at - now >= RESET_LONG_WINDOW_MS
            || (prev !== undefined && m.resets_at - prev.resetsAt >= RESET_LONG_WINDOW_MS);
          inferredLongWindow.set(key, { resetsAt: m.resets_at, long });
        }
      }
      if (!long) continue;
      if (m.resets_at > now) {
        if (soonest === null || m.resets_at < soonest) soonest = m.resets_at;
      } else if (m.resets_at === resetRetryFor && now - m.resets_at < 3 * 60_000) {
        providerLagging = true;
      }
    }
  }
  // The next reset moment and a provider-lag retry are both candidates —
  // keep exactly one pending timer, whichever lands first. Forced
  // usage-only refresh: the 60 s lastFetch guard would drop a plain one.
  const candidates: { at: number; kind: "moment" | "retry" }[] = [];
  if (soonest !== null) candidates.push({ at: soonest + 30_000, kind: "moment" });
  if (providerLagging) candidates.push({ at: now + 90_000, kind: "retry" });
  const pick = candidates.sort((a, b) => a.at - b.at)[0];
  if (pick) {
    const t = soonest;
    resetRefreshTimer = window.setTimeout(() => {
      resetRefreshTimer = undefined;
      // providerLagging only matches resets_at === resetRetryFor and the
      // retry clears it, so a lagging provider can retry exactly once.
      resetRetryFor = pick.kind === "moment" ? t : null;
      void refresh(true, true);
    }, Math.min(pick.at - now, 2_147_000_000));
  }
}

const logoPixels = new Map<string, number[]>();

async function rasterizeLogo(id: string): Promise<number[] | null> {
  const cached = logoPixels.get(id);
  if (cached) return cached;
  const svg = PROVIDER_ICONS[id];
  if (!svg) return null;

  const white = svg
    .replace(/fill="(?!none)[^"]*"/g, 'fill="#ffffff"')
    .replace(/stroke="(?!none)[^"]*"/g, 'stroke="#ffffff"');
  const url = URL.createObjectURL(new Blob([white], { type: "image/svg+xml" }));
  try {
    const img = new Image();
    await new Promise<void>((resolve, reject) => {
      img.onload = () => resolve();
      img.onerror = () => reject(new Error("svg load failed"));
      img.src = url;
    });
    const canvas = document.createElement("canvas");
    canvas.width = 32;
    canvas.height = 32;
    const ctx = canvas.getContext("2d")!;
    const scale = 28 / Math.max(img.width || 28, img.height || 28);
    const w = (img.width || 28) * scale;
    const h = (img.height || 28) * scale;
    ctx.drawImage(img, (32 - w) / 2, (32 - h) / 2, w, h);
    const pixels = Array.from(ctx.getImageData(0, 0, 32, 32).data);
    logoPixels.set(id, pixels);
    return pixels;
  } catch {
    return null;
  } finally {
    URL.revokeObjectURL(url);
  }
}

interface TraySyncState {
  revision: number | null;
  snapshots: Snapshot[];
  projection: TrayProjectionConfig;
}

let pendingTraySync: TraySyncState | null = null;
let traySyncRunning = false;
let traySyncFailureShown = false;
let traySyncFailureText = "";

function captureTraySyncState(): TraySyncState {
  const providerOrder = [...(config.layout?.providerOrder ?? [])];
  const providers: Record<string, TrayProjectionProvider> = {};
  for (const id of providerOrder) {
    const layout = liveProviderLayout(id);
    providers[id] = {
      metricOrder: [...layout.metricOrder],
      hidden: [...layout.hidden],
      starred: [...layout.starred],
    };
  }
  return {
    revision: lastUsageRevision,
    snapshots: lastSnapshots
      .filter((snapshot) => !isCardDisabled(snapshot.id))
      .map((snapshot) => ({
        ...snapshot,
        metrics: snapshot.metrics.map((metric) => ({ ...metric })),
      })),
    projection: {
      disabled: [...config.disabled],
      providerOrder,
      providers,
      pinned: config.pinned ? { ...config.pinned } : null,
      locale: resolveLocale(config.locale),
    },
  };
}

async function buildTrayStripEntries(state: TraySyncState): Promise<TrayStripEntry[]> {
  const entries: TrayStripEntry[] = [];
  for (const id of state.projection.providerOrder) {
    if (entries.length >= 4) break;
    if (isCardDisabled(id, state.projection.disabled)) continue;
    const layout = state.projection.providers[id];
    if (!layout?.starred.length) continue;
    const snapshot = state.snapshots.find((candidate) => candidate.id === id && candidate.status === "ok");
    if (!snapshot) continue;
    const starredMetrics = (layout.starred
      .filter((label) => !layout.hidden.includes(label))
      .map((label) =>
        snapshot.metrics.find((metric) => metric.label === label && metric.kind === "progress"),
      )
      .filter((metric): metric is Metric => Boolean(metric))
      .slice(0, 2));
    if (!starredMetrics.length) continue;
    const logo = await rasterizeLogo(providerFamily(id));
    if (!logo) continue;
    entries.push({ id, logo, labels: starredMetrics.map((metric) => metric.label) });
  }
  return entries;
}

function requestTraySync(): void {
  pendingTraySync = captureTraySyncState();
  if (!traySyncRunning) void drainTraySyncQueue();
}

async function drainTraySyncQueue(): Promise<void> {
  if (traySyncRunning) return;
  traySyncRunning = true;
  try {
    while (pendingTraySync) {
      const state = pendingTraySync;
      pendingTraySync = null;
      if (state.revision === null) continue;
      const entries = await buildTrayStripEntries(state);
      // Rasterizing a logo may yield while the user changes configuration.
      // Skip this stale generation before it reaches either native surface.
      if (pendingTraySync) continue;
      try {
        await invoke("sync_tray_surfaces", {
          revision: state.revision,
          projection: state.projection,
          entries,
        });
        if (traySyncFailureShown) {
          const status = document.querySelector("#status");
          if (status && status.textContent === traySyncFailureText) {
            status.textContent = configSaveError
              ? t(providerAutoRefreshUncertain ? "footer.autoRefreshUnknown" : "footer.configSaveFailed", { err: configSaveError })
              : "";
          }
        }
        traySyncFailureShown = false;
        traySyncFailureText = "";
      } catch (err) {
        if (!traySyncFailureShown) {
          const status = document.querySelector("#status");
          const message = t("footer.traySyncFailed", { err: String(err) });
          if (status) status.textContent = message;
          traySyncFailureShown = true;
          traySyncFailureText = message;
        }
      }
    }
  } finally {
    traySyncRunning = false;
    if (pendingTraySync) void drainTraySyncQueue();
  }
}

// ---------------------------------------------------------------------------
// Customize interactions
// ---------------------------------------------------------------------------

interface DragPayload {
  t: "row" | "provider";
  id: string;
  key?: string;
}

let dragPayload: DragPayload | null = null;

/// Rebuilds order + On-Demand membership after a row drop. The sequence is
/// [always..., DIVIDER, onDemand...]; where the row lands relative to the
/// divider decides which side it lives on.
function moveRow(L: ProviderLayout, key: string, target: string): void {
  const always = L.metricOrder.filter((k) => !L.onDemand.includes(k));
  const onDemand = L.metricOrder.filter((k) => L.onDemand.includes(k));
  const seq = [...always, DIVIDER, ...onDemand].filter((k) => k !== key);
  const at = target === DIVIDER ? seq.indexOf(DIVIDER) + 1 : seq.indexOf(target);
  if (at < 0) return;
  seq.splice(at, 0, key);
  const dividerIdx = seq.indexOf(DIVIDER);
  L.metricOrder = seq.filter((k) => k !== DIVIDER);
  L.onDemand = seq.slice(dividerIdx + 1).filter((k) => k !== DIVIDER);
}

function handleCustomizeClick(target: HTMLElement): boolean {
  const expand = target.closest<HTMLElement>("[data-cust-expand]");
  if (expand) {
    const id = expand.dataset.custExpand!;
    if (custExpanded.has(id)) {
      custExpanded.delete(id);
    } else {
      custExpanded.add(id);
    }
    // Toggle in place so the accordion animates instead of re-rendering.
    expand.closest(".customize-block")?.classList.toggle("open", custExpanded.has(id));
    expand.setAttribute("aria-expanded", String(custExpanded.has(id)));
    expand.closest(".customize-block")?.querySelector(".acc-body")?.toggleAttribute("inert", !custExpanded.has(id));
    return true;
  }
  const closeBtn = target.closest("[data-customize-close]");
  if (closeBtn) {
    setDrawer(false);
    return true;
  }
  const resetAll = target.closest("[data-reset-all]");
  if (resetAll) {
    void appConfirm({
      title: t("customize.resetTitle"),
      message: t("customize.resetBody"),
      confirmLabel: t("customize.resetConfirm"),
      danger: true,
    }).then((ok) => {
      if (!ok) return;
      // Reset presentation only. Keep both explicit grants and narrowing.
      config.layout = null;
      void patchConfig({ layout: null }).catch(() => {}).then(() => {
        setDrawer(false);
        void forceUsageRefreshAttempt(false).then(requestTraySync);
      });
    });
    return true;
  }
  const reset = target.closest<HTMLElement>("[data-reset]");
  if (reset && config.layout) {
    const id = reset.dataset.reset!;
    const snapshot = lastSnapshots.find((s) => s.id === id);
    const spend = accountSpend(id);
    config.layout.providers[id] = defaultProviderLayout(snapshot, spend, false);
    saveLayout();
    renderAll();
    return true;
  }
  const star = target.closest<HTMLElement>("[data-star]");
  if (star) {
    const [id, key] = star.dataset.star!.split("|");
    const L = providerLayout(id);
    if (liveProviderLayout(id).starred.includes(key)) {
      L.starred = L.starred.filter((k) => k !== key);
    } else if (liveProviderLayout(id).starred.length >= 2) {
      document.querySelector("#status")!.textContent = t("footer.twoStars");
      return true;
    } else {
      L.starred.push(key);
    }
    saveLayout();
    renderAll();
    return true;
  }
  return false;
}

function handleCustomizeChange(target: HTMLInputElement): void {
  if (target.dataset.providerAutoRefresh) {
    void changeProviderAutoRefresh(target.dataset.providerAutoRefresh, target.checked);
    return;
  }
  if (target.dataset.accessFamily) {
    if (target.checked) {
      custExpanded.add(target.dataset.accessFamily);
      const block = target.closest?.(".customize-block");
      block?.classList.add("open");
      block?.querySelector(".acc-body")?.removeAttribute("inert");
      block?.querySelector("[data-cust-expand]")?.setAttribute("aria-expanded", "true");
    }
    void changeProviderAccess("set_provider_family", { family: target.dataset.accessFamily, enabled: target.checked });
    return;
  }
  if (target.dataset.accessAccount) {
    void changeProviderAccess("set_provider_account", { account: target.dataset.accessAccount, enabled: target.checked });
    return;
  }
  if (target.dataset.providerMode) { void providerModeHandlers.change(target); return; }
  if (target.id === "stepfun-plan") {
    void patchConfig({ stepfunPlanCredits: target.value ? Number(target.value) : null }).then(() => refresh(true, true));
    return;
  }
  if (target.dataset.visible !== undefined) {
    const [id, key] = target.dataset.visible.split("|");
    const L = providerLayout(id);
    if (target.checked) L.hidden = L.hidden.filter((k) => k !== key);
    else if (!L.hidden.includes(key)) L.hidden.push(key);
    saveLayout();
  }
}

// Chromium's default drag snapshot on backdrop-filtered elements captures the
// glass layers behind the card too — a smeared ghost of the whole list. Hand
// it a small opaque pill instead and dim the real card while it's in flight.
let dragGhost: HTMLElement | null = null;

function setDragGhost(e: DragEvent, src: HTMLElement): void {
  const rect = src.getBoundingClientRect();
  const g = src.cloneNode(true) as HTMLElement;
  g.classList.add("drag-ghost");
  g.classList.remove("open"); // ghost of a provider card shows just its header bar
  g.setAttribute("inert", "");
  g.setAttribute("aria-hidden", "true");
  g.draggable = false;
  // The drag image is decorative, not another live settings form. Avoid
  // duplicate IDs, label references, focus stops and accidental submissions.
  for (const element of [g, ...g.querySelectorAll<HTMLElement>("*")]) {
    for (const attribute of ["id", "for", "name", "aria-controls", "aria-labelledby", "aria-describedby", "aria-owns"]) element.removeAttribute(attribute);
    if (element.matches("input, textarea, select, button")) (element as HTMLInputElement).disabled = true;
    if (element.matches("input, textarea, select, button, a[href], [tabindex]")) element.tabIndex = -1;
  }
  g.style.width = `${rect.width}px`;
  document.body.appendChild(g);
  e.dataTransfer?.setDragImage(g, e.clientX - rect.left, e.clientY - rect.top);
  dragGhost = g;
  requestAnimationFrame(() => src.classList.add("drag-src"));
}

function moveProviderOrder(order: string[], id: string, target: string): string[] {
  const isFamily = !id.includes("@");
  const moving = order.filter(item => isFamily ? item.split("@")[0] === id : item === id);
  const rest = order.filter(item => !moving.includes(item));
  const at = rest.findIndex(item => item === target || (!target.includes("@") && item.split("@")[0] === target));
  if (!moving.length || at < 0) return order;
  rest.splice(at, 0, ...moving);
  return rest;
}

function setupCustomizeDnD(providersEl: HTMLElement): void {
  providersEl.addEventListener("dragstart", (e) => {
    const row = (e.target as HTMLElement).closest<HTMLElement>("[data-cust-row]");
    if (row) {
      const [id, key] = row.dataset.custRow!.split("|");
      dragPayload = { t: "row", id, key };
      setDragGhost(e as DragEvent, row);
      e.stopPropagation();
      return;
    }
    const block = (e.target as HTMLElement).closest<HTMLElement>("[data-cust-provider]");
    if (block) {
      dragPayload = { t: "provider", id: block.dataset.custProvider! };
      setDragGhost(e as DragEvent, block);
    }
  });

  providersEl.addEventListener("dragend", () => {
    dragGhost?.remove();
    dragGhost = null;
    providersEl.querySelectorAll(".drag-src").forEach((el) => el.classList.remove("drag-src"));
  });

  providersEl.addEventListener("dragover", (e) => {
    if (dragPayload) e.preventDefault();
  });

  providersEl.addEventListener("drop", (e) => {
    if (!dragPayload) return;
    e.preventDefault();
    const target = e.target as HTMLElement;

    if (dragPayload.t === "row") {
      const L = providerLayout(dragPayload.id);
      const divider = target.closest<HTMLElement>("[data-divider]");
      const row = target.closest<HTMLElement>("[data-cust-row]");
      if (divider && divider.dataset.divider === dragPayload.id) {
        moveRow(L, dragPayload.key!, DIVIDER);
      } else if (row) {
        const [tid, tkey] = row.dataset.custRow!.split("|");
        if (tid === dragPayload.id && tkey !== dragPayload.key) moveRow(L, dragPayload.key!, tkey);
      }
      saveLayout();
      renderAll();
    } else if (config.layout) {
      const block = target.closest<HTMLElement>("[data-cust-provider]");
      if (block && block.dataset.custProvider !== dragPayload.id) {
        config.layout.providerOrder = moveProviderOrder(config.layout.providerOrder, dragPayload.id, block.dataset.custProvider!);
        saveLayout();
        renderAll();
      }
    }
    dragPayload = null;
    // renderAll() replaces the dragged node, so dragend may never bubble
    // back up — clean the ghost here too.
    dragGhost?.remove();
    dragGhost = null;
  });
}

// ---------------------------------------------------------------------------
// Settings pane
// ---------------------------------------------------------------------------

async function saveApiKey(provider: string): Promise<void> {
  const input = document.querySelector<HTMLInputElement>(`#key-${provider}`)!;
  const status = document.querySelector("#status")!;
  const submitted = input.value;
  // Saving or clearing a credential changes the identity a queued click
  // could reach. Use the same fence and write order as account grants.
  pendingAccessWrites += 1;
  refreshQueuedManualEpoch = null;
  cancelQueuedAccountRefreshes();
  const save = accessSaveQueue.then(async () => {
    await invoke("set_api_key", { provider, key: submitted });
    // This field may have been repainted while the save was pending. Clear
    // only the submitted draft, keeping anything typed after that request.
    const current = document.querySelector<HTMLInputElement>(`#key-${provider}`);
    if (current?.value === submitted) current.value = "";
    status.textContent = `${t(submitted.trim() ? "footer.keySaved" : "footer.keyRemoved", { name: providerDisplayName(provider) })} ${t("footer.keyEnableAccount")}`;
  }).finally(() => {
    // A key write can commit before cache cleanup or the IPC reply fails.
    // Never leave an old identity trusted when the final state is uncertain.
    accessEpoch += 1;
    lastUsageRevision = null;
    lastSnapshots = dropModeSnapshots(lastSnapshots, provider);
    accountRefreshErrors.delete(provider);
    pendingAccessWrites -= 1;
    renderProviderAccess();
    renderAll();
    requestTraySync();
  });
  accessSaveQueue = save.catch(() => {});
  try {
    await save;
    // This refresh is automatic: manual-only providers only replay a safe
    // saved snapshot or their first-query hint after backend invalidation.
    if (accountAuthorized(provider)) await forceUsageRefreshAttempt();
    requestTraySync();
  } catch (err) {
    status.textContent = t("footer.keySaveFailed", { err: String(err) });
  }
}

function accountAuthorized(id: string): boolean {
  return config.accessPolicy.enabledFamilies.includes(providerFamily(id)) &&
    config.accessPolicy.enabledAccounts.includes(id);
}

let accessSaveQueue: Promise<unknown> = Promise.resolve();
let accessEpoch = 0;
let pendingAccessWrites = 0;
function changeProviderAccess(command: string, input: Record<string, unknown> | (() => Record<string, unknown>)): Promise<void> {
  // Fence manual intent when the write is requested, not when its reply
  // arrives: the backend may already have committed the new authority.
  pendingAccessWrites += 1;
  refreshQueuedManualEpoch = null;
  cancelQueuedAccountRefreshes();
  const save = accessSaveQueue.then(async () => {
    const args = typeof input === "function" ? input() : input;
    const echoed = await invoke<Config & { accessWarning?: string }>(command, args);
    if (echoed.accessWarning) document.querySelector("#status")!.textContent = t((command === "set_scan_source" || command === "configure_scan_source") ? "footer.scanAccessWarning" : "footer.providerAccessWarning", { err: echoed.accessWarning });
    config.accessPolicy = echoed.accessPolicy;
    accessEpoch += 1;
    lastUsageRevision = null;
    lastSpend = [];
    spendLoaded = false;
    if (command === "set_scan_source" || command === "configure_scan_source") void scanSourceCatalog.load();
    renderScanSources();
    // Old frontend opt-out filters can only narrow consent; explicit enable
    // clears the corresponding presentation filter after the backend grant.
    if (command === "set_provider_account" && args.enabled === true) {
      await patchConfig({ disabled: config.disabled.filter((id) => id !== args.account) });
    }
    if (command === "set_provider_region" && typeof args.id === "string") {
      lastSnapshots = dropModeSnapshots(lastSnapshots, args.id);
      const accepted = config.accessPolicy.regions[args.id];
      const form = Array.from(document.querySelectorAll<HTMLFormElement>("[data-provider-local]"))
        .find(item => item.dataset.providerLocal === args.id);
      const origin = form?.querySelector<HTMLInputElement>("[data-local-origin]");
      if (origin && accepted?.startsWith("local:")) {
        try {
          // A submitted trailing slash/whitespace is the same saved origin,
          // not a fresh draft. Keep genuinely newer, different edits intact.
          if (localModeSelection(origin.value) === accepted) {
            origin.value = accepted.slice(6);
            origin.defaultValue = origin.value;
          }
        } catch { /* A newer incomplete draft remains editable. */ }
      }
    }
    lastSnapshots = lastSnapshots.filter((snapshot) => accountAuthorized(snapshot.id));
    if (!accountAuthorized("moonshot")) {
      for (const snapshot of lastSnapshots) if (snapshot.id === "kimi") {
        snapshot.metrics = snapshot.metrics.filter((m) => !["API", "Credits used", "Balance", "Vouchers", "Cash"].includes(m.label));
      }
    }
    renderProviderAccess();
    renderAll();
    requestTraySync();
    if (command !== "discover_provider_account") void forceUsageRefreshAttempt(false);
  }).finally(() => { pendingAccessWrites -= 1; });
  accessSaveQueue = save.catch((err) => {
    document.querySelector("#status")!.textContent = (command === "set_scan_source" || command === "configure_scan_source")
      ? t("footer.scanAccessError", { err: displayScanError(String(err)) })
      : t("footer.providerAccessError", { err: String(err) });
    renderScanSources();
    renderProviderAccess();
    renderAll();
  });
  return save.catch(() => {});
}

const providerModeCatalog = createProviderModeCatalog(
  () => invoke<ProviderModeCatalogEntry[]>("get_provider_modes"),
  renderProviderAccess,
);
const providerModeHandlers = createProviderModeHandlers(
  () => providerModeCatalog.state.entries,
  changeProviderAccess,
  (key) => { document.querySelector("#status")!.textContent = t(key); },
);

function preserveSettingsFocus(root: Element): () => void {
  const active = root.ownerDocument?.activeElement as HTMLElement | null;
  if (!active || !root.contains(active)) return () => {};
  const identity = active.getAttributeNames().filter(name => name === "id" || name.startsWith("data-"))
    .map(name => [name, active.getAttribute(name)] as const);
  if (!identity.length) return () => {};
  const tag = active.tagName;
  const field = active as HTMLInputElement | HTMLTextAreaElement;
  const start = field.selectionStart;
  const end = field.selectionEnd;
  const direction = field.selectionDirection;
  const scrollTop = active.scrollTop;
  const scrollLeft = active.scrollLeft;
  return () => {
    const next = Array.from(root.querySelectorAll<HTMLElement>("input, textarea, select, button"))
      .find(item => item.tagName === tag && identity.every(([name, value]) => item.getAttribute(name) === value));
    if (!next || next.closest("[inert], [hidden]") || next.hasAttribute("disabled")) return;
    next.focus({ preventScroll: true });
    if (typeof start === "number" && typeof end === "number") {
      (next as HTMLInputElement | HTMLTextAreaElement).setSelectionRange(start, end, direction ?? undefined);
    }
    next.scrollTop = scrollTop;
    next.scrollLeft = scrollLeft;
  };
}

function renderProviderAccess(preserveDrafts = true): void {
  const root = document.querySelector("#provider-access");
  if (!root) return;
  const restoreFocus = preserveDrafts ? preserveSettingsFocus(root) : () => {};
  // Repainting a backend response must not discard keys/directories being
  // edited. These values stay in the live form only; never read credentials.
  const drafts = new Map((preserveDrafts ? Array.from(root.querySelectorAll<HTMLInputElement>("[data-provider-draft]")) : []).filter(input => input.value !== input.defaultValue).map((input) => [input.dataset.providerDraft!, input.value]));
  root.innerHTML = renderCustomize();
  root.querySelectorAll<HTMLInputElement>("[data-provider-draft]").forEach((input) => { if (drafts.has(input.dataset.providerDraft!)) input.value = drafts.get(input.dataset.providerDraft!)!; });
  restoreFocus();
}

function populatePinnedOptions(): void {
  const select = document.querySelector<HTMLSelectElement>("#pinned")!;
  const current = config.pinned ? `${config.pinned.provider}::${config.pinned.label}` : "";
  select.replaceChildren(new Option(t("settings.pinAuto"), ""));
  for (const s of lastSnapshots) {
    if (isCardDisabled(s.id) || s.status !== "ok") continue;

    for (const m of s.metrics) {
      if (m.kind !== "progress") continue;
      const value = `${s.id}::${m.label}`;
      select.add(
        new Option(
          t("settings.pinOption", { name: s.name, label: displayMetricLabel(m.label) }),
          value,
          false,
          value === current,
        ),
      );
    }
  }
}

function applyLocale(): void {
  config.locale = normalizeLocalePref(config.locale);
  setActiveLocale(resolveLocale(config.locale));
  applyStaticI18n();
  void applyWidgetState(); // its button hints are stateful, not data-i18n
  applyAppearance();
  const status = document.querySelector("#status");
  if (status) {
    if (lastSnapshots.length) {
      const time = new Date().toLocaleTimeString(localeTag(), {
        hour: "2-digit",
        minute: "2-digit",
      });
      status.textContent = t("footer.updated", { time });
    } else {
      status.textContent = t("footer.starting");
    }
  }
  if (lastSnapshots.length || lastSpend.length) renderIfVisible();
  populatePinnedOptions();
  renderProviderAccess();
  renderScanSources(); // × buttons re-translate with the rest of the list
  renderPriceSuccess();
  renderBuildInfo();
}

const SCAN_SOURCES: readonly [string, string][] = [
  ["claude", "Claude Code"],
  ["codex", "Codex"],
  ["opencode", "OpenCode"],
  ["pi", "Pi / oh-my-pi"],
  ["stepcode", "Step Code"],
  ["grok", "Grok CLI"],
  ["devin", "Devin CLI"],
  ["minimax", "MiniMax"],
  ["hermes", "Hermes"],
  ["kimi", "Kimi Code"],
  ["qwen", "Qwen Code"],
];
const scanSourceCatalog = createLogSourceCatalog(
  () => invoke<ScanSourceStatus[]>("get_scan_sources"), renderScanSources, () => accessEpoch,
);
function currentScanSources(): ScanSourceStatus[] {
  return projectScanSources(SCAN_SOURCES.map(([source]) => source), config.accessPolicy, scanSourceCatalog.state.entries);
}

const scanSourceHandlers = createLogSourceHandlers(currentScanSources, changeProviderAccess);
function renderScanSources(preserveDrafts = true): void {
  const root = document.querySelector("#scan-sources");
  if (!root) return;
  const restoreFocus = preserveDrafts ? preserveSettingsFocus(root) : () => {};
  const editingCustom = new Set((preserveDrafts ? Array.from(root.querySelectorAll<HTMLSelectElement>("[data-scan-mode]")) : []).filter(input => input.value === "custom").map(input => input.dataset.scanMode!));
  const drafts = new Map((preserveDrafts ? Array.from(root.querySelectorAll<HTMLTextAreaElement>("[data-scan-draft]")) : []).filter(input => input.value !== input.defaultValue).map(input => [input.dataset.scanDraft!, input.value]));
  root.innerHTML = renderLogSourceSettings({ ...scanSourceCatalog.state, entries: currentScanSources() }, SCAN_SOURCES, t, escapeHtml);
  root.querySelectorAll<HTMLTextAreaElement>("[data-scan-draft]").forEach(input => {
    if (drafts.has(input.dataset.scanDraft!)) input.value = drafts.get(input.dataset.scanDraft!)!;
    if (editingCustom.has(input.dataset.scanDraft!)) {
      const form = input.closest<HTMLFormElement>("[data-scan-custom]");
      if (form) form.hidden = false;
      const select = input.closest(".scan-source")?.querySelector<HTMLSelectElement>("[data-scan-mode]");
      if (select) select.value = "custom";
    }
  });
  restoreFocus();
}

function renderPriceSuccess(): void {
  const element = document.querySelector("#price-last-success");
  if (element) element.textContent = lastPriceSync === null ? t("pricing.loading")
    : lastPriceSync.last_success_ms
      ? t("pricing.lastSuccess", { time: new Date(lastPriceSync.last_success_ms).toLocaleString(localeTag()) })
      : t("pricing.noSuccess");
  const status = document.querySelector("#price-sync-status");
  if (status) status.textContent = priceSyncStatus ? t(priceSyncStatus.key, priceSyncStatus.vars) : "";
}
function syncPrices(): Promise<void> {
  if (priceSync) return priceSync;
  const button = document.querySelector<HTMLButtonElement>("#sync-prices")!;
  const requestAccessEpoch = accessEpoch;
  button.disabled = true;
  button.setAttribute("aria-busy", "true");
  priceSyncStatus = { key: "pricing.syncing" };
  renderPriceSuccess();
  priceSync = (async () => {
    try {
      const result = await invoke<PricingUpdate>("sync_prices");
      ++priceEpoch; // Discard every outstanding pre-publication spend response.
      lastPriceSync = result.pricing;
      renderPriceSuccess();
      if (requestAccessEpoch === accessEpoch) {
        lastSpend = mergeRepricedLocal(lastSpend, result.spend.rows, result.spend.revision, spendVisible);
        spendLoaded = true;
        renderIfVisible();
      }
      const missing = lastSpend.filter(spendVisible).reduce((sum, row) => sum + row.unpriced, 0);
      priceSyncStatus = { key: missing > 0 ? "pricing.syncedMissing" : "pricing.synced" };
      renderPriceSuccess();
    } catch (error) {
      priceSyncStatus = { key: "pricing.failed", vars: { err: String(error) } };
      renderPriceSuccess();
      // A completed atomic publication may precede a failed local scan.
      try {
        const current = await invoke<PricingStatus>("pricing_status");
        if (current.catalog_stamp !== lastPriceSync?.catalog_stamp) ++priceEpoch;
        lastPriceSync = current;
        renderPriceSuccess();
      } catch { /* keep last verified status */ }
    } finally {
      button.disabled = false;
      button.setAttribute("aria-busy", "false");
      priceSync = null;
    }
  })();
  return priceSync;
}

async function initSettings(): Promise<void> {
  config = await invoke<Config>("get_config");
  confirmedProviderAutoRefresh = normalizeProviderAutoRefresh(config.providerAutoRefresh);
  config.providerAutoRefresh = confirmedProviderAutoRefresh;
  // Optional metadata never gates consent controls, reset or general settings.
  // Each catalog owns its loading/error/retry state independently.
  void providerModeCatalog.load();
  void scanSourceCatalog.load();
  document.querySelector("#sync-prices")!.addEventListener("click", () => void syncPrices());
  const initialPriceEpoch = priceEpoch;
  void invoke<PricingStatus>("pricing_status").then((status) => { if (!priceSync && acceptPriceEpoch(initialPriceEpoch, priceEpoch)) { lastPriceSync = status; renderPriceSuccess(); } }).catch(() => {});
  renderProviderAccess();
  // First launch timestamp — backs the star prompt's "a few days old"
  // eligibility. Recorded once, on the first config load that finds it 0.
  if (!config.firstSeenMs) {
    void patchConfig({ firstSeenMs: Date.now() }).catch(() => {});
  }
  config.locale = normalizeLocalePref(config.locale);
  try {
    const sys = await invoke<string>("system_ui_locale");
    setSystemLocale(sys === "zh" || sys === "ru" ? sys : "en");
  } catch {
    // Dev / missing command — fall back to navigator.language.
  }
  applyLocale();
  if (["today", "yesterday", "last30"].includes(config.spendTab)) {
    spendTab = config.spendTab;
  }

  const interval = document.querySelector<HTMLInputElement>("#interval")!;
  interval.value = String(config.refreshMinutes);
  interval.addEventListener("change", () => {
    const minutes = Math.max(1, Math.min(120, Number(interval.value) || 5));
    interval.value = String(minutes);
    void patchConfig({ refreshMinutes: minutes }).then(scheduleAutoRefresh);
  });

  const autostart = document.querySelector<HTMLInputElement>("#autostart")!;
  autostart.checked = await invoke<boolean>("get_autostart");
  autostart.addEventListener("change", () => {
    void invoke("set_autostart", { enabled: autostart.checked }).catch((err) => {
      document.querySelector("#status")!.textContent = t("footer.autostartFailed", { err: String(err) });
      autostart.checked = !autostart.checked;
    });
  });

  const pacing = document.querySelector<HTMLInputElement>("#pacing")!;
  pacing.checked = config.pacingAlways;
  pacing.addEventListener("change", () => {
    void patchConfig({ pacingAlways: pacing.checked }).then(renderAll);
  });

  const timeFormat = document.querySelector<HTMLSelectElement>("#timeformat")!;
  timeFormat.value = config.timeFormat;
  timeFormat.addEventListener("change", () => {
    void patchConfig({ timeFormat: timeFormat.value as Config["timeFormat"] }).then(renderAll);
  });

  const localeSel = document.querySelector<HTMLSelectElement>("#locale")!;
  localeSel.value = config.locale;
  localeSel.addEventListener("change", () => {
    const next = normalizeLocalePref(localeSel.value);
    void patchConfig({ locale: next }).catch(() => {});
    applyLocale();
    requestTraySync();
  });

  const notifyToggles: [string, keyof Config][] = [
    ["#notify-reset", "notifyReset"],
    ["#notify-almost", "notifyAlmostOut"],
    ["#notify-close", "notifyCuttingClose"],
    ["#notify-runout", "notifyWillRunOut"],
  ];
  for (const [selector, key] of notifyToggles) {
    const box = document.querySelector<HTMLInputElement>(selector)!;
    box.checked = Boolean(config[key]);
    box.addEventListener("change", () => {
      void patchConfig({ [key]: box.checked } as Partial<Config>);
      // notifyReset also arms/clears the reset-moment refresh timer.
      if (key === "notifyReset") scheduleResetRefresh();
    });
  }

  const pinned = document.querySelector<HTMLSelectElement>("#pinned")!;
  pinned.addEventListener("change", () => {
    const [provider, label] = pinned.value.split("::");
    const value = provider && label ? { provider, label } : null;
    void patchConfig({ pinned: value }).catch(() => {});
    requestTraySync();
  });

  const showSpend = document.querySelector<HTMLInputElement>("#show-total-spend")!;
  showSpend.checked = config.showTotalSpend;
  showSpend.addEventListener("change", () => {
    void patchConfig({ showTotalSpend: showSpend.checked }).then(renderAll);
  });

  applyAppearance();
  const appearance = document.querySelector<HTMLSelectElement>("#appearance")!;
  appearance.value = config.appearance;
  appearance.addEventListener("change", () => {
    void patchConfig({ appearance: appearance.value as Config["appearance"] }).then(applyAppearance);
  });

  const density = document.querySelector<HTMLInputElement>("#density")!;
  density.checked = config.density === "compact";
  density.addEventListener("change", () => {
    void patchConfig({ density: density.checked ? "compact" : "regular" }).then(applyAppearance);
  });

  const minimal = document.querySelector<HTMLInputElement>("#minimal")!;
  minimal.checked = config.minimal === true;
  minimal.addEventListener("change", () => {
    void patchConfig({ minimal: minimal.checked }).then(() => {
      applyAppearance();
      renderAll();
    });
  });

  const glass = document.querySelector<HTMLInputElement>("#glass")!;
  glass.checked = config.glassEffects !== false;
  glass.addEventListener("change", () => {
    // Widget see-through is applied Rust-side, so the widget state must
    // be re-synced after the glass flag flips too.
    void patchConfig({ glassEffects: glass.checked }).then(applyGlass).then(applyWidgetState);
  });
  applyGlass();

  const reduceAnim = document.querySelector<HTMLInputElement>("#reduce-anim")!;
  reduceAnim.checked = config.reduceAnimations === true;
  reduceAnim.addEventListener("change", () => {
    void patchConfig({ reduceAnimations: reduceAnim.checked }).then(applyReduceMotion);
  });
  applyReduceMotion();

  const shortcut = document.querySelector<HTMLInputElement>("#shortcut")!;
  shortcut.value = config.shortcut;
  shortcut.addEventListener("change", async () => {
    const status = document.querySelector("#status")!;
    try {
      await invoke("set_shortcut", { shortcut: shortcut.value });
      await patchConfig({ shortcut: shortcut.value });
      status.textContent = shortcut.value.trim() ? t("footer.shortcutSaved") : t("footer.shortcutCleared");
    } catch (err) {
      status.textContent = `${err}`;
    }
  });

  const proxyEnabled = document.querySelector<HTMLInputElement>("#proxy-enabled")!;
  const proxyUrl = document.querySelector<HTMLInputElement>("#proxy-url")!;
  proxyEnabled.checked = config.proxy?.enabled ?? false;
  proxyUrl.value = config.proxy?.url ?? "";
  const saveProxy = () => {
    void patchConfig({ proxy: { enabled: proxyEnabled.checked, url: proxyUrl.value.trim() } }).then(
      () => {
        document.querySelector("#status")!.textContent = t("footer.proxySaved");
      },
    );
  };
  proxyEnabled.addEventListener("change", saveProxy);
  proxyUrl.addEventListener("change", saveProxy);

  const scanSources = document.querySelector("#scan-sources")!;
  scanSources.addEventListener("submit", (event) => {
    event.preventDefault();
    void scanSourceHandlers.submit(event.target as HTMLFormElement);
  });
  scanSources.addEventListener("change", (event) => {
    const target = event.target as HTMLInputElement;
    if (target.dataset.scanMode && target.value === "custom") {
      const form = target.closest(".scan-source")?.querySelector<HTMLFormElement>("[data-scan-custom]");
      if (form) { form.hidden = false; form.querySelector<HTMLTextAreaElement>("[data-scan-paths]")?.focus(); }
    }
    void scanSourceHandlers.change(target);
  });
  scanSources.addEventListener("click", (event) => {
    if ((event.target as Element).closest("[data-scan-retry]")) void scanSourceCatalog.load();
  });
  renderScanSources();

  populatePinnedOptions();

  document.querySelector("#reset-all-settings")!.addEventListener("click", () => {
    void resetAllSettings();
  });


  initWidget({ getConfig: () => config, patchConfig, brandColor: spendColor });
}

/// Restore every preference to the same defaults a fresh install gets.
/// API keys, lastSeenVersion, and welcomeDismissed stay (keys are not
/// "settings"; What's-new shouldn't pop again).
async function resetAllSettings(): Promise<void> {
  const ok = await appConfirm({
    title: t("settings.resetTitle"),
    message: t("settings.resetBody"),
    confirmLabel: t("settings.resetConfirm"),
    danger: true,
  });
  if (!ok) return;
  pendingAccessWrites += 1;
  refreshQueuedManualEpoch = null;
  cancelQueuedAccountRefreshes();
  // Reset is the last consent write after every operation queued before its
  // confirmation. Delayed source responses cannot restore grants afterward.
  const resetAccess = accessSaveQueue.then(async () => {
    const echoed = await invoke<Config>("reset_provider_access");
    config.accessPolicy = echoed.accessPolicy;
    accessEpoch += 1;
    lastUsageRevision = null;
    lastSnapshots = [];
    lastSpend = [];
    spendLoaded = false;
    scanSourceCatalog.clear();
    renderScanSources(false);
    renderProviderAccess(false);
    // Metadata cannot delay the accepted revocation or repopulate old drafts.
    void scanSourceCatalog.load();
    renderAll();
    requestTraySync();
  }).finally(() => { pendingAccessWrites -= 1; });
  // A failed reset leaves accepted grants intact and must not poison future
  // consent actions. The original promise below still reports the failure.
  accessSaveQueue = resetAccess.catch(() => {});
  try {
    await resetAccess;
  } catch (err) {
    document.querySelector("#status")!.textContent = t("footer.accessResetFailed", { err: String(err) });
    return;
  }
  try {
    await invoke("set_autostart", { enabled: true });
  } catch {
    // Dev builds skip autostart; the preference is still saved below.
  }
  try {
    await invoke("set_shortcut", { shortcut: "" });
  } catch {
    // Invalid leftover shortcut shouldn't block the rest of the reset.
  }
  await patchConfig({
    refreshMinutes: 1,
    disabled: [],
    pinned: null,
    trayProviders: [],
    pacingAlways: true,
    notifyAlmostOut: true,
    notifyCuttingClose: true,
    notifyWillRunOut: true,
    notifyReset: true,
    spendTab: "today",
    spendMetric: "cost",
    showUsed: false,
    resetExact: false,
    timeFormat: "auto",
    layout: null,
    appearance: "dark",
    density: "compact",
    minimal: false,
    glassEffects: true,
    shortcut: "",
    proxy: { enabled: false, url: "" },
    showTotalSpend: true,
    reduceAnimations: false,
    locale: "auto",
    stepfunPlanCredits: null,
    widgetMode: false,
    widgetCollapsed: false,
    widgetLocked: false,
    codexExtraDirs: [],
  }).catch(() => {});
  await saveProviderAutoRefresh({ reset: true });
  spendTab = "today";
  applyLocale();
  syncSettingsControls();
  scheduleAutoRefresh();
  applyAppearance();
  applyGlass();
  applyReduceMotion();
  void applyWidgetState();
  document.body.classList.remove("settings-open");
  document.querySelector("#settings-btn")?.classList.remove("active");
  void forceUsageRefreshAttempt(false).then(requestTraySync);
}

function syncSettingsControls(): void {
  renderProviderAccess();
  const setNum = (sel: string, v: string) => {
    const el = document.querySelector<HTMLInputElement>(sel);
    if (el) el.value = v;
  };
  const setCheck = (sel: string, v: boolean) => {
    const el = document.querySelector<HTMLInputElement>(sel);
    if (el) el.checked = v;
  };
  const setSelect = (sel: string, v: string) => {
    const el = document.querySelector<HTMLSelectElement>(sel);
    if (el) el.value = v;
  };
  setNum("#interval", String(config.refreshMinutes));
  setCheck("#pacing", config.pacingAlways);
  setSelect("#timeformat", config.timeFormat);
  setSelect("#locale", config.locale);
  setSelect(
    "#stepfun-plan",
    config.stepfunPlanCredits == null ? "" : String(config.stepfunPlanCredits),
  );
  setCheck("#notify-reset", config.notifyReset);
  setCheck("#notify-almost", config.notifyAlmostOut);
  setCheck("#notify-close", config.notifyCuttingClose);
  setCheck("#notify-runout", config.notifyWillRunOut);
  setCheck("#show-total-spend", config.showTotalSpend);
  setSelect("#appearance", config.appearance);
  setCheck("#density", config.density === "compact");
  setCheck("#minimal", config.minimal === true);
  setCheck("#glass", config.glassEffects !== false);
  setCheck("#reduce-anim", config.reduceAnimations === true);
  setNum("#shortcut", config.shortcut);
  setCheck("#proxy-enabled", config.proxy?.enabled ?? false);
  setNum("#proxy-url", config.proxy?.url ?? "");
  const autostart = document.querySelector<HTMLInputElement>("#autostart");
  if (autostart) autostart.checked = true;
  renderScanSources();
  populatePinnedOptions();
  // Resetting toggles programmatically fires no change events — re-arm
  // (or clear) the reset-moment timer against the restored values.
  scheduleResetRefresh();
}

// ---------------------------------------------------------------------------
// Boot
// ---------------------------------------------------------------------------

window.addEventListener("DOMContentLoaded", () => {
  const appLogo = document.querySelector<HTMLElement>("#app-logo")!;
  appLogo.innerHTML = `<img src="${paneLogo}" alt="Pane" />`;
  // Party mode, the easy way: triple-click the logo. (The Konami code
  // still works, for the culture.)
  let logoClicks = 0;
  let logoClickReset: number | undefined;
  appLogo.addEventListener("click", () => {
    logoClicks += 1;
    window.clearTimeout(logoClickReset);
    logoClickReset = window.setTimeout(() => (logoClicks = 0), 1200);
    if (logoClicks >= 3) {
      logoClicks = 0;
      toggleParty();
    }
  });
  document.querySelector("#theme-btn")!.addEventListener("click", toggleTheme);
  setupTrailFisheye();
  setupTooltips();
  // No lens init here: applyGlass() (via initSettings, after the saved
  // config arrives) owns it — a fixed timer raced the config load and
  // built the maps even for users who turned glass off.
  window.addEventListener("keydown", (e) => {
    konamiListen(e);
    if (e.ctrlKey && e.key.toLowerCase() === "z" && customizeOpen) {
      e.preventDefault();
      undoLayout();
    }
    // Esc backs out of Customize/Settings; on the dashboard it hides the
    // popover (Mac parity). IME candidate cancel must not close anything.
    if (e.key === "Escape" && !e.isComposing && e.keyCode !== 229) {
      if (customizeOpen || document.body.classList.contains("settings-open")) {
        setDrawer(false);
        setSettings(false);
      } else {
        void invoke("hide_popover");
      }
    }
    // Ctrl+R refreshes data — and must NOT reload the webview.
    if (e.ctrlKey && e.key.toLowerCase() === "r") {
      e.preventDefault();
      void refresh(true, false, "userRefresh");
    }
  });
  void getVersion().then((v) => {
    appVersion = v;
    buildText = `v${v} · build ${__BUILD_STAMP__}`;
    renderBuildInfo();

  });
  document.querySelector("#refresh")!.addEventListener("click", () => void refresh(true, false, "userRefresh"));

  const setSettings = (open: boolean) => {
    document.body.classList.toggle("settings-open", open);
    document.querySelector("#settings-btn")?.classList.toggle("active", open);
  };
  document.querySelector("#settings-btn")!.addEventListener("click", () => {
    setDrawer(false);
    setSettings(!document.body.classList.contains("settings-open"));
  });
  document.querySelector("#settings-close")!.addEventListener("click", () => setSettings(false));
  document.querySelector("#changelog-btn")!.addEventListener("click", () => {
    setSettings(false);
    showChangelogDialog(t("dialog.changelog"), parseChangelog());
  });
  document.querySelectorAll<HTMLElement>(".acc-head").forEach((head) => {
    head.addEventListener("click", () => head.parentElement!.classList.toggle("open"));
  });
  document.querySelector("#customize-btn")!.addEventListener("click", () => {
    setSettings(false);
    setDrawer(!customizeOpen);
  });
  const drawerBody = document.querySelector<HTMLElement>("#drawer-body")!;
  drawerBody.addEventListener("click", (e) => {
    const target = e.target as HTMLElement;
    if (target.closest("[data-provider-modes-retry]")) { void providerModeCatalog.load(); return; }
    const save = target.closest<HTMLElement>("[data-save]");
    if (save?.dataset.save) { void saveApiKey(save.dataset.save); return; }
    handleCustomizeClick(target);
  });
  drawerBody.addEventListener("change", (e) => handleCustomizeChange(e.target as HTMLInputElement));
  drawerBody.addEventListener("submit", (event) => {
    event.preventDefault();
    const form = event.target as HTMLFormElement;
    if (form.dataset.providerLocal) { void providerModeHandlers.submit(form); return; }
    const family = form.dataset.accessDiscover;
    const directory = form.querySelector<HTMLInputElement>("[data-access-directory]")?.value.trim();
    if (family && directory) void changeProviderAccess("discover_provider_account", { family, directory });
  });
  setupCustomizeDnD(drawerBody);

  const providersEl = document.querySelector<HTMLElement>("#providers")!;
  // The donut center toggles what the card meters: dollars ⇄ raw tokens.
  // Left or right click both work; the choice persists.
  const toggleSpendMetric = (back = false) => {
    config.spendMetric = nextSpendMetric(back);
    void patchConfig({ spendMetric: config.spendMetric });
    renderAll();
  };
  providersEl.addEventListener("contextmenu", (e) => {
    if ((e.target as Element).closest?.(".donut-wrap")) {
      e.preventDefault();
      toggleSpendMetric(true); // right-click cycles backward
    }
  });

  // Donut hover: pointing at a segment or its legend row swells the arc
  // and dims the others, Mac-style. [data-pid] links the two.
  const setDonutHot = (id: string | null) => {
    document.querySelectorAll<HTMLElement>(".total-spend [data-pid]").forEach((el) => {
      el.classList.toggle("hot", id !== null && el.dataset.pid === id);
    });
  };
  providersEl.addEventListener("mouseover", (e) => {
    const t = (e.target as Element).closest?.<HTMLElement>(".total-spend [data-pid]");
    if (t) setDonutHot(t.dataset.pid ?? null);
  });
  providersEl.addEventListener("mouseout", (e) => {
    if ((e.target as Element).closest?.(".total-spend [data-pid]")) setDonutHot(null);
  });

  // In-popover reordering: drag a card by the grip in its header. The new
  // order saves to the same layout Customize edits, so both stay in sync.
  let dragCard: HTMLElement | null = null;
  let armedCard: HTMLElement | null = null;
  providersEl.addEventListener("mousedown", (e) => {
    const grip = (e.target as HTMLElement).closest(".drag-grip, .drag-handle");
    const card = grip?.closest<HTMLElement>("article[data-provider]");
    if (card) {
      card.draggable = true;
      armedCard = card;
    }
  });
  // A grip press that never turns into a drag would otherwise leave the
  // card grab-anywhere; disarm on release when no drag started.
  document.addEventListener("mouseup", () => {
    if (armedCard && !dragCard) armedCard.draggable = false;
    armedCard = null;
  });
  providersEl.addEventListener("dragstart", (e) => {
    dragCard = (e.target as HTMLElement).closest?.("article[data-provider]") ?? null;
    dragCard?.classList.add("dragging");
  });
  providersEl.addEventListener("dragover", (e) => {
    if (!dragCard) return;
    e.preventDefault();
    const over = (e.target as HTMLElement).closest?.<HTMLElement>("article[data-provider]");
    if (!over || over === dragCard) return;
    const r = over.getBoundingClientRect();
    const before = e.clientY < r.top + r.height / 2;
    over.parentElement!.insertBefore(dragCard, before ? over : over.nextElementSibling);
  });
  const endCardDrag = () => {
    if (!dragCard) return;
    dragCard.classList.remove("dragging");
    dragCard.draggable = false;
    dragCard = null;
    ensureLayout();
    const domIds = Array.from(
      providersEl.querySelectorAll<HTMLElement>("article[data-provider]")
    ).map((a) => a.dataset.provider!);
    const L = config.layout!;
    L.providerOrder = [...domIds, ...L.providerOrder.filter((id) => !domIds.includes(id))];
    void patchConfig({ layout: L });
    requestTraySync();
    updateTrailActive();
  };
  providersEl.addEventListener("drop", (e) => {
    e.preventDefault();
    endCardDrag();
  });
  providersEl.addEventListener("dragend", endCardDrag);

  // In-card row reordering: press a row and pull it 5 px vertically to
  // lift it. Pointer Events — not HTML5 drag — so the gesture never
  // collides with the card-grip drag above, and until the lift every
  // click/hover target inside the row keeps working normally.
  providersEl.addEventListener("pointerdown", (e) => {
    if (e.button !== 0 || config.minimal) return;
    const target = e.target as HTMLElement;
    if (
      target.closest(
        "button, a, input, select, textarea, .quick-links, .card-caret, .provider-head",
      )
    )
      return;
    const row = target.closest<HTMLElement>(".card-panel [data-row]");
    const card = row?.closest<HTMLElement>("article[data-provider]");
    if (!row || !card) return;
    if (rowDrag) finishRowDrag(rowDrag, false); // a press that never released
    rowDrag = {
      id: card.dataset.provider!,
      card,
      row,
      pointerId: e.pointerId,
      startY: e.clientY,
      offsetY: e.clientY - row.getBoundingClientRect().top,
      lifted: false,
      fromDemand: !!row.parentElement?.classList.contains("on-demand"),
      placeholder: null,
      originParent: null,
      originNext: null,
    };
  });
  document.addEventListener("pointermove", (e) => {
    const d = rowDrag;
    if (!d || e.pointerId !== d.pointerId) return;
    if (!d.lifted) {
      if (Math.abs(e.clientY - d.startY) < 5) return;
      liftRowDrag(d);
    }
    moveRowDrag(d, e.clientY);
    e.preventDefault();
  });
  document.addEventListener("pointerup", (e) => {
    const d = rowDrag;
    if (!d || e.pointerId !== d.pointerId) return;
    finishRowDrag(d, true);
  });
  document.addEventListener("pointercancel", (e) => {
    const d = rowDrag;
    if (!d || e.pointerId !== d.pointerId) return;
    finishRowDrag(d, false);
  });
  document.addEventListener("keydown", (e) => {
    if (e.key !== "Escape" || !rowDrag) return;
    const lifted = rowDrag.lifted;
    finishRowDrag(rowDrag, false);
    // Only a real drag swallows Esc — otherwise it still closes the
    // popover via the window-level Escape handler.
    if (lifted) e.stopPropagation();
  });

  providersEl.addEventListener("click", (e) => {
    const target = e.target as HTMLElement;
    const accountRefresh = target.closest<HTMLButtonElement>("[data-account-refresh]");
    if (accountRefresh) {
      const id = accountRefresh.dataset.accountRefresh!;
      if (!accountRefresh.disabled) void refresh(true, true, "userRefresh", { kind: "account", accountId: id });
      return;
    }

    const link = target.closest<HTMLElement>("[data-link]");
    if (link) {
      void invoke("open_link", { url: link.dataset.link }).catch((err) => {
        document.querySelector("#status")!.textContent = t("footer.openLinkFailed", { err: String(err) });
      });
      return;
    }
    const shareBtn = target.closest<HTMLElement>("[data-share]");
    if (shareBtn) {
      void shareCard(shareBtn.dataset.share!);
      return;
    }
    if (target.closest(".donut-wrap")) {
      toggleSpendMetric();
      return;
    }
    if (target.closest("[data-welcome-close]")) {
      config.welcomeDismissed = true;
      void patchConfig({ welcomeDismissed: true });
      renderAll();
      return;
    }
    if (target.closest("[data-welcome-customize]")) {
      config.welcomeDismissed = true;
      void patchConfig({ welcomeDismissed: true });
      renderAll();
      setDrawer(true);
      return;
    }
    const tab = target.closest("[data-tab]");
    if (tab) {
      switchSpendTab(tab.getAttribute("data-tab") as SpendTab);
      return;
    }
    const caret = target.closest<HTMLElement>("[data-caret]");
    if (caret) {
      const id = caret.dataset.caret!;
      const L = providerLayout(id);
      L.expanded = !L.expanded;
      saveLayout(false);
      animateExpandId = L.expanded ? id : null;
      renderAll();
      animateExpandId = null;
      return;
    }
    const flip = target.closest<HTMLElement>("[data-flip]");
    if (flip) {
      if (flip.dataset.flip === "usage") {
        config.showUsed = !config.showUsed;
        void patchConfig({ showUsed: config.showUsed });
      } else {
        config.resetExact = !config.resetExact;
        void patchConfig({ resetExact: config.resetExact });
      }
      renderAll();
    }
  });


  const tip = document.querySelector<HTMLElement>("#model-tip")!;
  providersEl.addEventListener("mouseover", (e) => {
    if (customizeOpen) return;
    const target = e.target as HTMLElement;
    const resets = target.closest<HTMLElement>("[data-resets]");
    if (resets) {
      resetsPopover.inlineEnter(resets);
      return;
    }
    const bar = target.closest<HTMLElement>("[data-trend]");
    if (bar) {
      showTrendTip(bar);
      return;
    }
    const cap = target.closest<HTMLElement>("[data-cap]");
    if (cap) {
      showCapacityTip(cap);
      return;
    }
    const row = target.closest<HTMLElement>("[data-spend]");
    if (row) showModelTip(row);
  });
  providersEl.addEventListener("mouseout", (e) => {
    const target = e.target as HTMLElement;
    const to = e.relatedTarget as HTMLElement | null;
    const resets = target.closest<HTMLElement>("[data-resets]");
    if (resets && (!to || !resets.contains(to))) resetsPopover.inlineLeave();
    const hovered = target.closest<HTMLElement>("[data-spend], [data-trend], [data-cap]");
    if (hovered && (!to || !hovered.contains(to))) tip.hidden = true;
  });
  let scrollRaf = 0;
  providersEl.addEventListener("scroll", () => {
    tip.hidden = true;
    resetsPopover.onScroll();
    cancelAnimationFrame(scrollRaf);
    scrollRaf = requestAnimationFrame(updateTrailActive);
  });

  const rsPop = document.querySelector<HTMLElement>("#resets-pop")!;
  rsPop.addEventListener("mouseenter", () => resetsPopover.detailEnter());
  rsPop.addEventListener("mouseleave", () => resetsPopover.detailLeave());
  rsPop.addEventListener("mouseover", (e) => {
    const node = (e.target as HTMLElement).closest<HTMLElement>(".rs-node");
    resetsPopover.nodeHover(node?.dataset.rsCredit ?? null);
  });
  rsPop.addEventListener("mouseout", (e) => {
    const node = (e.target as HTMLElement).closest<HTMLElement>(".rs-node");
    const to = e.relatedTarget as HTMLElement | null;
    if (node && (!to || !node.contains(to))) resetsPopover.nodeHover(null);
  });
  rsPop.addEventListener("click", (e) => resetsPopover.click(e.target as HTMLElement));

  document.querySelector("#trail")!.addEventListener("click", (e) => {
    const tick = (e.target as HTMLElement).closest<HTMLElement>("[data-trail]");
    if (!tick) return;
    const card = trailCards()[Number(tick.dataset.trail)];
    card?.scrollIntoView({ behavior: reduceMotion() ? "auto" : "smooth", block: "start" });
  });

  // A pending star roll must not present into a hidden window.
  document.addEventListener("visibilitychange", () => {
    if (document.hidden && starPromptTimer !== undefined) {
      window.clearTimeout(starPromptTimer);
      starPromptTimer = undefined;
    }
  });

  void listen("popover-shown", () => {

    // Always reopen on the main page, at the top — leftover Customize/
    // Settings panels, a stale confirm dialog, or a stale scroll position
    // from the previous visit feel like the app is stuck mid-page.
    setDrawer(false);
    setSettings(false);
    dismissConfirm?.();
    dismissWhatsNew?.();
    dismissStarPrompt?.();
    if (starPromptTimer !== undefined) {
      window.clearTimeout(starPromptTimer);
      starPromptTimer = undefined;
    }
    resetsPopover.dismiss();
    // A fresh update's notes present on the first open after launch.
    if (pendingWhatsNew) {
      showChangelogDialog(t("dialog.whatsNew", { version: appVersion }), pendingWhatsNew);
      pendingWhatsNew = null;
    }
    // The star prompt only rolls when nothing else is presenting and the
    // welcome card is gone.
    if (
      config.welcomeDismissed &&
      !pendingWhatsNew &&
      !dismissWhatsNew &&
      !dismissConfirm
    ) {
      maybeShowStarPrompt();
    }
    // Replay any renders skipped while hidden, before the reveal plays.
    if (pendingRender) {
      pendingRender = false;
      renderAll();
      populatePinnedOptions();
    }
    providersEl.scrollTop = 0;
    updateTrailActive();
    if (lastSnapshots.length && !customizeOpen) playReveal();
    requestTraySync();
    void refresh();
  });
  void initSettings().then(() => {
    scheduleAutoRefresh();
    void paintCachedSnapshots();
    void refresh(true);
    // Queued, not shown: the window is usually still hidden in the tray at
    // startup — the first popover-shown presents it. Runs after the config
    // load so lastSeenVersion is the real stored value, not the default.
    void getVersion().then((v) => {
      pendingWhatsNew = computeWhatsNew(v);
    });
  });

  // Countdown texts ("Resets in 3h 41m") tick every 30 s — but only for
  // eyes that can see them; hidden ticks fold into the deferred render.
  setInterval(() => {
    if ((lastSnapshots.length || lastSpend.length) && !customizeOpen) renderIfVisible();
  }, 30_000);
});
