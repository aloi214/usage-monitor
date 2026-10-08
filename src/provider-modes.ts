export interface ProviderModeCatalogEntry {
  id: string;
  family: string;
  choices: readonly { value: string; label: string }[];
  selected: string | null;
  defaultSelection: string | null;
  allowLocalOrigin: boolean;
}
type Translate = (key: string, vars?: Record<string, string | number>) => string;
type ChangeAccess = (command: string, input: Record<string, unknown>) => Promise<void>;

export interface ProviderModeCatalogState {
  entries: ProviderModeCatalogEntry[];
  loading: boolean;
  error: string | null;
}

// One startup read or explicit user retry. Repaints never trigger requests,
// and repeated clicks share the active read instead of creating a retry loop.
export function createProviderModeCatalog(
  read: () => Promise<ProviderModeCatalogEntry[]>,
  repaint: () => void,
) {
  const state: ProviderModeCatalogState = { entries: [], loading: false, error: null };
  let active: Promise<void> | null = null;
  return {
    state,
    load(): Promise<void> {
      if (active) return active;
      state.loading = true;
      active = Promise.resolve().then(read).then((entries) => {
        state.entries = entries;
        state.error = null;
      }).catch((error) => {
        state.error = String(error);
      }).finally(() => {
        state.loading = false;
        active = null;
        repaint();
      });
      repaint();
      return active;
    },
  };
}

export function renderProviderModeCatalogStatus(
  state: ProviderModeCatalogState,
  t: Translate,
  escape: (value: string) => string,
): string {
  if (state.error !== null) {
    return `<div class="settings-group"><p class="settings-note" role="alert">${escape(t("settings.modeLoadFailed", { err: state.error }))}</p><button type="button" class="mini-btn" data-provider-modes-retry${state.loading ? " disabled" : ""}>${escape(t(state.loading ? "settings.modeRetrying" : "settings.modeRetry"))}</button></div>`;
  }
  return state.loading ? `<p class="settings-note" role="status">${escape(t("settings.modeLoading"))}</p>` : "";
}

// This check gives immediate feedback. The backend still validates every value
// against the trusted account binding and its production destination policy.
export function localModeSelection(input: string): string {
  const url = new URL(input.trim());
  if (!["http:", "https:"].includes(url.protocol) ||
      !["localhost", "127.0.0.1", "[::1]"].includes(url.hostname) ||
      url.username || url.password || url.pathname !== "/" || url.search || url.hash || url.port === "0") {
    throw new Error("settings.modeInvalidLocal");
  }
  return `local:${url.origin}`;
}

export function createProviderModeHandlers(
  getCatalog: () => readonly ProviderModeCatalogEntry[],
  changeAccess: ChangeAccess,
  reportError: (key: string) => void,
) {
  return {
    async change(target: Pick<HTMLSelectElement, "dataset" | "value">): Promise<void> {
      const id = target.dataset.providerMode;
      if (!id) return;
      const mode = getCatalog().find((entry) => entry.id === id);
      if (!mode || !mode.choices.some((choice) => choice.value === target.value)) {
        reportError(target.value ? "settings.modeInvalid" : "settings.modeMissing");
        return;
      }
      await changeAccess("set_provider_region", { id, region: target.value });
    },
    async submit(form: HTMLFormElement): Promise<void> {
      const id = form.dataset.providerLocal;
      if (!id) return;
      const mode = getCatalog().find((entry) => entry.id === id);
      if (!mode || mode.family !== "ollama" || !mode.allowLocalOrigin) {
        reportError("settings.modeInvalid");
        return;
      }
      let region: string;
      try {
        region = localModeSelection(form.querySelector<HTMLInputElement>("[data-local-origin]")?.value ?? "");
      } catch {
        reportError("settings.modeInvalidLocal");
        return;
      }
      await changeAccess("set_provider_region", { id, region });
    },
  };
}

export function renderProviderMode(
  entry: ProviderModeCatalogEntry | undefined,
  selected: string | undefined,
  t: Translate,
  escape: (value: string) => string,
): string {
  if (!entry) return "";
  let value = selected ?? entry.defaultSelection ?? "";
  if (!entry.choices.some((choice) => choice.value === value)) {
    try {
      if (!entry.allowLocalOrigin || !value.startsWith("local:")) throw new Error();
      value = localModeSelection(value.slice(6));
    } catch { value = ""; }
  }
  const options = entry.choices.map((choice) => {
    const familyKey = `mode.option.${entry.family}.${choice.value}`;
    const key = t(familyKey) === familyKey ? `mode.option.${choice.value}` : familyKey;
    const label = t(key) === key ? choice.label : t(key);
    return `<option value="${escape(choice.value)}"${choice.value === value ? " selected" : ""}>${escape(label)}</option>`;
  }).join("");
  const custom = entry.allowLocalOrigin && value.startsWith("local:") && !entry.choices.some((choice) => choice.value === value);
  const currentCustom = custom ? `<option value="${escape(value)}" selected>${escape(t("settings.modeCustom"))}: ${escape(value.slice(6))}</option>` : "";
  const local = entry.allowLocalOrigin
    ? `<form class="key-row" data-provider-local="${escape(entry.id)}"><input data-local-origin data-provider-draft="origin-${escape(entry.id)}" aria-label="${escape(t("settings.modeLocalOrigin"))}" placeholder="http://127.0.0.1:11434" value="${escape(value.startsWith("local:") ? value.slice(6) : "")}" required /><button type="submit">${escape(t("settings.modeSaveLocal"))}</button></form><p class="settings-note">${escape(t("settings.modeLocalHelp"))}</p>`
    : "";
  const guidance = !value ? t("settings.modeMissing") : t("settings.modeNoGrant");
  const qwen = entry.family === "qwen" ? `<p class="settings-note">${escape(t("settings.modeQwenHelp"))}</p>` : "";
  return `<div class="provider-mode"><label class="setting-row"><span>${escape(t("settings.modeLabel"))}</span><select data-provider-mode="${escape(entry.id)}" aria-label="${escape(t("settings.modeLabel"))}"><option value="" disabled${value ? "" : " selected"}>${escape(t("settings.modeChoose"))}</option>${options}${currentCustom}</select></label><p class="settings-note">${escape(guidance)}</p>${!value ? `<p class="settings-note">${escape(t("settings.modeNoGrant"))}</p>` : ""}${qwen}${local}</div>`;
}

export function dropModeSnapshots<T extends { id: string }>(snapshots: T[], id: string): T[] {
  return snapshots.filter((snapshot) => snapshot.id !== id && !(id === "moonshot" && snapshot.id === "kimi"));
}
