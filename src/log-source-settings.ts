export interface ScanSourceStatus {
  source: string;
  enabled: boolean;
  mode: "default" | "custom";
  customDirectories: string[];
  directories: { path: string; status: "found" | "notFound" | "unavailable" | "unverified"; error?: string }[];
  error?: string;
}
export interface ScanSourceSettings {
  enabled: boolean;
  mode: "default" | "custom";
  customDirectories: string[];
  defaultDirectories: string[];
}
// Permission intent always comes from the latest accepted config. Metadata
// can describe a matching path, but can never grant access or select a mode.
export function projectScanSources(
  sources: readonly string[],
  policy: { scanSources?: Record<string, ScanSourceSettings>; scanRoots: Record<string, string[]> },
  metadata: ScanSourceStatus[],
): ScanSourceStatus[] {
  return sources.map(source => {
    const legacy = policy.scanRoots[source] ?? [];
    const accepted = policy.scanSources?.[source] ?? {
      enabled: legacy.length > 0,
      mode: legacy.length ? "custom" as const : "default" as const,
      customDirectories: [...legacy], defaultDirectories: [],
    };
    const entry = metadata.find(item => item.source === source);
    const selected = accepted.mode === "custom" ? accepted.customDirectories : accepted.defaultDirectories;
    const matches = entry?.mode === accepted.mode &&
      JSON.stringify(entry.customDirectories) === JSON.stringify(accepted.customDirectories) &&
      JSON.stringify(entry.directories.map(item => item.path)) === JSON.stringify(selected);
    const preview = !accepted.enabled && accepted.mode === "default" && !selected.length &&
      entry?.mode === "default" && !entry.enabled;
    const directories: ScanSourceStatus["directories"] = matches || preview ? entry!.directories
      : selected.map(path => ({ path, status: "unverified" }));
    return { source, enabled: accepted.enabled, mode: accepted.mode, customDirectories: [...accepted.customDirectories],
      directories, error: matches || preview ? entry?.error : undefined };
  });
}

interface SourceCatalogState { entries: ScanSourceStatus[]; loading: boolean; error: string | null; }
type Translate = (key: string, vars?: Record<string, string | number>) => string;
type ChangeAccess = (command: string, input: Record<string, unknown> | (() => Record<string, unknown>)) => Promise<void>;

// Metadata-only catalog. New authorization epochs supersede older reads, and
// an error waits for explicit retry instead of starting a background loop.
export function createLogSourceCatalog(read: () => Promise<ScanSourceStatus[]>, repaint: () => void, epoch: () => number) {
  const state: SourceCatalogState = { entries: [], loading: false, error: null };
  let active: { epoch: number; request: Promise<void> } | null = null;
  let sequence = 0;
  return {
    state,
    clear(): void {
      ++sequence;
      active = null;
      state.entries = [];
      state.error = null;
      state.loading = false;
    },
    load(): Promise<void> {
      const at = epoch();
      if (active?.epoch === at) return active.request;
      const requestId = ++sequence;
      state.loading = true;
      const request = Promise.resolve().then(read).then(entries => {
        if (requestId === sequence && at === epoch()) { state.entries = entries; state.error = null; }
      }).catch(error => {
        if (requestId === sequence && at === epoch()) state.error = String(error);
      }).finally(() => {
        if (requestId === sequence) { state.loading = false; active = null; repaint(); }
      });
      active = { epoch: at, request };
      repaint();
      return request;
    },
  };
}

export function createLogSourceHandlers(entries: () => ScanSourceStatus[], changeAccess: ChangeAccess) {
  return {
    async change(target: Pick<HTMLInputElement, "dataset" | "checked" | "value">): Promise<void> {
      const source = target.dataset.scanEnabled ?? target.dataset.scanMode;
      if (!source || !entries().some(entry => entry.source === source)) return;
      if (target.dataset.scanMode && target.value !== "default") return; // custom applies only on Save
      const enabledEdit = target.dataset.scanEnabled !== undefined;
      const enabled = target.checked;
      await changeAccess("configure_scan_source", () => {
        const saved = entries().find(entry => entry.source === source)!;
        return { source, enabled: enabledEdit ? enabled : saved.enabled, mode: enabledEdit ? saved.mode : "default" };
      });
    },
    async submit(form: HTMLFormElement): Promise<void> {
      const source = form.dataset.scanCustom;
      if (!source || !entries().some(entry => entry.source === source)) return;
      const directories = [...new Set((form.querySelector<HTMLTextAreaElement>("[data-scan-paths]")?.value ?? "").split(/\r?\n/).map(value => value.trim()).filter(Boolean))];
      await changeAccess("configure_scan_source", () => ({ source, enabled: entries().find(entry => entry.source === source)!.enabled, mode: "custom", directories }));
    },
  };
}

export function renderLogSourceSettings(state: SourceCatalogState, sources: readonly (readonly [string, string])[], t: Translate, escape: (value: string) => string): string {
  const status = state.error ? `<p class="settings-note" role="alert">${escape(t("settings.scanLoadFailed", { err: state.error }))}</p>` : state.loading ? `<p class="settings-note" role="status">${escape(t("settings.scanLoading"))}</p>` : "";
  return status + `<button type="button" class="mini-btn" data-scan-retry${state.loading ? " disabled" : ""}>${escape(t("settings.scanRecheck"))}</button>` + sources.map(([source, name]) => {
    const entry = state.entries.find(item => item.source === source);
    if (!entry) return `<div class="settings-group"><strong>${escape(name)}</strong><p class="settings-note">${escape(t("settings.scanStatusUnknown"))}</p></div>`;
    const paths = entry.directories.map(directory => `<div class="scan-path"><span class="path">${escape(directory.path)}</span><span class="platform-status">${escape(t(directory.status === "found" ? "settings.scanFound" : directory.status === "notFound" ? "settings.scanNotFound" : directory.status === "unverified" ? "settings.scanUnverified" : "settings.scanUnavailable"))}</span>${directory.error ? `<p class="settings-note" role="alert">${escape(directory.error)}</p>` : ""}</div>`).join("");
    return `<section class="settings-group scan-source" data-scan-source="${source}"><div class="setting-row"><strong>${escape(name)}</strong><label class="toggle"><input type="checkbox" data-scan-enabled="${source}" aria-label="${escape(t("settings.scanToggle", { name }))}"${entry.enabled ? " checked" : ""} /> ${escape(t(entry.enabled ? "settings.scanOn" : "settings.scanOff"))}</label></div><label class="setting-row"><span>${escape(t("settings.scanLocation"))}</span><select data-scan-mode="${source}"><option value="default"${entry.mode === "default" ? " selected" : ""}>${escape(t("settings.scanDefault"))}</option><option value="custom"${entry.mode === "custom" ? " selected" : ""}>${escape(t("settings.scanCustom"))}</option></select></label><p class="settings-note">${escape(t(entry.enabled ? "settings.scanEffective" : "settings.scanRemembered"))}: ${escape(t(entry.mode === "default" ? "settings.scanDefault" : "settings.scanCustom"))}</p>${paths || `<p class="settings-note">${escape(t("settings.scanNoLocations"))}</p>`}${entry.error ? `<p class="settings-note" role="alert">${escape(entry.error)}</p>` : ""}<p class="settings-note">${escape(t(`settings.scanHint.${source}`))}</p><form class="scan-custom-form" data-scan-custom="${source}"${entry.mode === "custom" ? "" : " hidden"}><label for="scan-paths-${source}">${escape(t("settings.scanCustomDirectories"))}</label><textarea id="scan-paths-${source}" data-scan-paths data-scan-draft="${source}" rows="2" spellcheck="false" placeholder="${escape(t("settings.scanDirectoryPlaceholder"))}">${escape(entry.customDirectories.join("\n"))}</textarea><p class="settings-note">${escape(t("settings.scanCustomHelp"))}</p><button type="submit" class="mini-btn">${escape(t("settings.scanSaveCustom"))}</button></form></section>`;
  }).join("");
}
