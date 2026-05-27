/**
 * Safety fader WebviewViewProvider.
 *
 * Renders the 5-detent SAFETY fader in a VSCode webview view, reads the
 * current state from `.conductr`, and writes back on user interaction.
 *
 * Resolution logic stays in `conductr-core` (Rust).  This panel only reads
 * and writes user-pinned values; it never re-derives the maturity default.
 * The maturity-derived suggestion is surfaced by calling `conductr safety
 * suggest` (CLI, to be implemented) and the subtitle reflects divergence.
 */
import * as vscode from 'vscode';
import * as path from 'path';
import {
    clearSafetyOverride,
    PRESET_DISPLAY,
    PRESET_TAGLINE,
    readSafetyConfig,
    SAFETY_PRESETS,
    SAFETY_ROLES,
    SafetyConfig,
    SafetyPreset,
    SafetyRole,
    writeSafetyOverride,
    writeSafetyPreset,
} from './conductrConfig';

// ── Message types (webview ↔ host) ────────────────────────────────────────────

interface SetPresetMessage {
    type: 'setPreset';
    preset: SafetyPreset;
}

interface SetOverrideMessage {
    type: 'setOverride';
    role: SafetyRole;
    preset: SafetyPreset;
}

interface ClearOverrideMessage {
    type: 'clearOverride';
    role: SafetyRole;
}

type WebviewMessage = SetPresetMessage | SetOverrideMessage | ClearOverrideMessage;

// ── Panel ──────────────────────────────────────────────────────────────────────

export class SafetyFaderPanel implements vscode.WebviewViewProvider {
    static readonly viewType = 'conductr.safetyFader';

    private _view?: vscode.WebviewView;

    constructor(private readonly _context: vscode.ExtensionContext) {}

    resolveWebviewView(
        webviewView: vscode.WebviewView,
        _context: vscode.WebviewViewResolveContext,
        _token: vscode.CancellationToken,
    ): void {
        this._view = webviewView;

        webviewView.webview.options = {
            enableScripts: true,
        };

        webviewView.webview.onDidReceiveMessage(
            (message: WebviewMessage) => this._handleMessage(message),
            undefined,
            this._context.subscriptions,
        );

        // Refresh when `.conductr` changes on disk.
        const watcher = vscode.workspace.createFileSystemWatcher('**/.conductr');
        watcher.onDidChange(() => this._refresh());
        watcher.onDidCreate(() => this._refresh());
        this._context.subscriptions.push(watcher);

        this._refresh();
    }

    // ── Private ───────────────────────────────────────────────────────────────

    private _repoRoot(): string | undefined {
        return vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
    }

    private _refresh(): void {
        if (!this._view) { return; }
        const root = this._repoRoot();
        const config = root ? readSafetyConfig(root) : { preset: undefined, overrides: {} };
        this._view.webview.html = buildHtml(config);
    }

    private _handleMessage(message: WebviewMessage): void {
        const root = this._repoRoot();
        if (!root) { return; }

        switch (message.type) {
            case 'setPreset':
                writeSafetyPreset(root, message.preset);
                this._refresh();
                break;
            case 'setOverride':
                writeSafetyOverride(root, message.role, message.preset);
                this._refresh();
                break;
            case 'clearOverride':
                clearSafetyOverride(root, message.role);
                this._refresh();
                break;
        }
    }
}

// ── HTML builder ───────────────────────────────────────────────────────────────

function buildHtml(config: SafetyConfig): string {
    const activePreset = config.preset ?? 'fast';
    const tagline = PRESET_TAGLINE[activePreset];

    const detentButtons = SAFETY_PRESETS.map((p) => {
        const isActive = p === activePreset;
        return `<button
      class="detent${isActive ? ' active' : ''}"
      data-preset="${p}"
      title="${PRESET_TAGLINE[p]}"
    >${PRESET_DISPLAY[p]}</button>`;
    }).join('\n');

    const overrideRows = SAFETY_ROLES.map((role) => {
        const override = config.overrides[role];
        const miniDetents = SAFETY_PRESETS.map((p) => {
            const isActive = p === (override ?? activePreset);
            const isPinned = p === override;
            return `<button
        class="mini-detent${isActive ? ' active' : ''}${isPinned ? ' pinned' : ''}"
        data-role="${role}"
        data-preset="${p}"
        title="${PRESET_TAGLINE[p]}"
      >${PRESET_DISPLAY[p][0]}</button>`;
        }).join('');

        const clearBtn = override
            ? `<button class="clear-btn" data-role="${role}" title="Clear override (revert to project preset)">✕</button>`
            : '';

        return `<tr>
      <td class="role-name">${role}</td>
      <td class="mini-fader">${miniDetents}</td>
      <td>${clearBtn}</td>
    </tr>`;
    }).join('\n');

    return `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<style>
  :root {
    --vscode-font-family: var(--vscode-editor-font-family, monospace);
    --accent: var(--vscode-button-background, #0e7af4);
    --accent-fg: var(--vscode-button-foreground, #fff);
    --bg: var(--vscode-editor-background, #1e1e1e);
    --fg: var(--vscode-editor-foreground, #d4d4d4);
    --border: var(--vscode-panel-border, #3c3c3c);
  }
  body {
    font-family: var(--vscode-font-family);
    color: var(--fg);
    background: var(--bg);
    padding: 12px;
    margin: 0;
    font-size: 12px;
  }
  h3 { margin: 0 0 8px; font-size: 13px; letter-spacing: 0.05em; }

  /* ── Global fader ── */
  .fader {
    display: flex;
    gap: 4px;
    margin-bottom: 6px;
  }
  .detent {
    flex: 1;
    padding: 6px 2px;
    border: 1px solid var(--border);
    background: transparent;
    color: var(--fg);
    cursor: pointer;
    font-size: 10px;
    font-family: var(--vscode-font-family);
    border-radius: 3px;
    transition: background 0.1s;
  }
  .detent:hover { background: var(--vscode-list-hoverBackground, #2a2d2e); }
  .detent.active {
    background: var(--accent);
    color: var(--accent-fg);
    border-color: var(--accent);
    font-weight: bold;
  }
  .tagline {
    font-style: italic;
    opacity: 0.75;
    margin-bottom: 16px;
    font-size: 11px;
  }

  /* ── Per-routine overrides drawer ── */
  details { border-top: 1px solid var(--border); padding-top: 8px; }
  summary {
    cursor: pointer;
    font-size: 11px;
    opacity: 0.8;
    user-select: none;
    margin-bottom: 6px;
  }
  table { width: 100%; border-collapse: collapse; }
  td { padding: 3px 0; vertical-align: middle; }
  .role-name { width: 90px; font-size: 11px; opacity: 0.85; }
  .mini-fader { display: flex; gap: 2px; }
  .mini-detent {
    width: 20px;
    height: 20px;
    border: 1px solid var(--border);
    background: transparent;
    color: var(--fg);
    cursor: pointer;
    font-size: 9px;
    font-family: var(--vscode-font-family);
    border-radius: 2px;
    padding: 0;
  }
  .mini-detent:hover { background: var(--vscode-list-hoverBackground, #2a2d2e); }
  .mini-detent.active { background: var(--vscode-list-activeSelectionBackground, #094771); }
  .mini-detent.pinned {
    background: var(--accent);
    color: var(--accent-fg);
    border-color: var(--accent);
    font-weight: bold;
  }
  .clear-btn {
    background: transparent;
    border: none;
    color: var(--fg);
    cursor: pointer;
    opacity: 0.5;
    padding: 0 4px;
    font-size: 11px;
  }
  .clear-btn:hover { opacity: 1; }
</style>
</head>
<body>
<h3>SAFETY</h3>
<div class="fader">
  ${detentButtons}
</div>
<p class="tagline">"${tagline}"</p>

<details>
  <summary>Per-routine overrides ▾</summary>
  <table>
    ${overrideRows}
  </table>
</details>

<script>
  const vscode = acquireVsCodeApi();

  document.querySelectorAll('.detent').forEach(btn => {
    btn.addEventListener('click', () => {
      vscode.postMessage({ type: 'setPreset', preset: btn.dataset.preset });
    });
  });

  document.querySelectorAll('.mini-detent').forEach(btn => {
    btn.addEventListener('click', () => {
      vscode.postMessage({ type: 'setOverride', role: btn.dataset.role, preset: btn.dataset.preset });
    });
  });

  document.querySelectorAll('.clear-btn').forEach(btn => {
    btn.addEventListener('click', () => {
      vscode.postMessage({ type: 'clearOverride', role: btn.dataset.role });
    });
  });
</script>
</body>
</html>`;
}
