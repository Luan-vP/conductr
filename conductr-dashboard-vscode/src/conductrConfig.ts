/**
 * Low-level `.conductr` TOML reader/writer for the safety section.
 *
 * Writes use a line-patch strategy (same as the Rust config module) so
 * comments and unrelated sections are preserved.
 *
 * Resolution logic lives in `conductr-core` (Rust). This module only reads
 * and writes what the user has explicitly pinned; it never re-derives the
 * maturity default on its own.
 */
import * as fs from 'fs';
import * as path from 'path';

// ── Types (mirror conductr-core::safety) ──────────────────────────────────────

export type SafetyPreset = 'unhinged' | 'feral' | 'fast' | 'strict' | 'bureaucratic';
export type SafetyRole =
    | 'architect'
    | 'implementer'
    | 'reviewer'
    | 'tester'
    | 'security'
    | 'doc-writer'
    | 'idle-sweeper';

export const SAFETY_PRESETS: SafetyPreset[] = [
    'unhinged',
    'feral',
    'fast',
    'strict',
    'bureaucratic',
];

export const SAFETY_ROLES: SafetyRole[] = [
    'architect',
    'implementer',
    'reviewer',
    'tester',
    'security',
    'doc-writer',
    'idle-sweeper',
];

export const PRESET_DISPLAY: Record<SafetyPreset, string> = {
    unhinged:     'UNHINGED',
    feral:        'FERAL',
    fast:         'FAST',
    strict:       'STRICT',
    bureaucratic: 'BUREAUCRATIC',
};

export const PRESET_TAGLINE: Record<SafetyPreset, string> = {
    unhinged:     "Ship it; we'll debug in prod",
    feral:        'First to green wins',
    fast:         'Trust the process; keep moving',
    strict:       'Measure twice, cut once',
    bureaucratic: "If it's worth shipping, it's worth a paper trail",
};

export interface SafetyConfig {
    /** Globally-pinned preset; absent → follow maturity default. */
    preset: SafetyPreset | undefined;
    /** Per-routine overrides; absent key → follow global preset. */
    overrides: Partial<Record<SafetyRole, SafetyPreset>>;
}

// ── Reader ────────────────────────────────────────────────────────────────────

/**
 * Read `[safety]` and `[safety.overrides]` from the workspace `.conductr`.
 * Returns defaults when the file or section is absent.
 */
export function readSafetyConfig(repoRoot: string): SafetyConfig {
    const filePath = path.join(repoRoot, '.conductr');
    if (!fs.existsSync(filePath)) {
        return { preset: undefined, overrides: {} };
    }
    const content = fs.readFileSync(filePath, 'utf8');
    return parseSafetyFromToml(content);
}

function parseSafetyFromToml(content: string): SafetyConfig {
    const result: SafetyConfig = { preset: undefined, overrides: {} };
    const lines = content.split('\n');

    let inSafety = false;
    let inOverrides = false;

    for (const line of lines) {
        const t = line.trim();

        if (t === '[safety]') {
            inSafety = true;
            inOverrides = false;
            continue;
        }
        if (t === '[safety.overrides]') {
            inSafety = false;
            inOverrides = true;
            continue;
        }
        // Any other section header exits both blocks.
        if (t.startsWith('[') && !t.startsWith('[[') && t.endsWith(']')) {
            inSafety = false;
            inOverrides = false;
            continue;
        }

        if (inSafety) {
            const m = t.match(/^preset\s*=\s*"([^"]+)"/);
            if (m) {
                result.preset = m[1] as SafetyPreset;
            }
        }
        if (inOverrides) {
            const m = t.match(/^([\w-]+)\s*=\s*"([^"]+)"/);
            if (m) {
                result.overrides[m[1] as SafetyRole] = m[2] as SafetyPreset;
            }
        }
    }

    return result;
}

// ── Writers ───────────────────────────────────────────────────────────────────

/** Pin (or update) the global safety preset in `.conductr`. */
export function writeSafetyPreset(repoRoot: string, preset: SafetyPreset): void {
    const filePath = path.join(repoRoot, '.conductr');
    const content = fs.existsSync(filePath) ? fs.readFileSync(filePath, 'utf8') : '';
    const updated = patchSectionKey(content, '[safety]', 'preset', `preset = "${preset}"`);
    fs.writeFileSync(filePath, updated, 'utf8');
}

/** Pin (or update) a per-routine safety override in `.conductr`. */
export function writeSafetyOverride(
    repoRoot: string,
    role: SafetyRole,
    preset: SafetyPreset,
): void {
    const filePath = path.join(repoRoot, '.conductr');
    const content = fs.existsSync(filePath) ? fs.readFileSync(filePath, 'utf8') : '';
    const keyLine = `${role.padEnd(12)} = "${preset}"`;
    const updated = patchSectionKey(content, '[safety.overrides]', role, keyLine);
    fs.writeFileSync(filePath, updated, 'utf8');
}

/** Remove a per-routine safety override from `.conductr`. */
export function clearSafetyOverride(repoRoot: string, role: SafetyRole): void {
    const filePath = path.join(repoRoot, '.conductr');
    if (!fs.existsSync(filePath)) { return; }
    const content = fs.readFileSync(filePath, 'utf8');
    const updated = patchSectionKey(content, '[safety.overrides]', role, null);
    fs.writeFileSync(filePath, updated, 'utf8');
}

/**
 * Generic section-key patcher — mirrors `patch_section_key` in Rust config.rs.
 *
 * `newLine = string` → upsert the key.
 * `newLine = null`   → delete the key (clear override).
 */
function patchSectionKey(
    content: string,
    sectionHeader: string,
    key: string,
    newLine: string | null,
): string {
    const lines = content.split('\n');
    const trailingNewline = content.endsWith('\n');

    // Remove the phantom empty string left by split when content ends with '\n'.
    const workLines = trailingNewline ? lines.slice(0, -1) : [...lines];

    let sectionStart = -1;
    let nextSection = -1;

    for (let i = 0; i < workLines.length; i++) {
        const t = workLines[i].trim();
        if (t === sectionHeader) {
            sectionStart = i;
        } else if (
            sectionStart >= 0 &&
            t.startsWith('[') &&
            !t.startsWith('[[') &&
            t.endsWith(']') &&
            t !== sectionHeader
        ) {
            nextSection = i;
            break;
        }
    }

    if (sectionStart >= 0) {
        const end = nextSection >= 0 ? nextSection : workLines.length;
        const result: string[] = [];
        let keyFound = false;

        for (let i = 0; i < workLines.length; i++) {
            if (i > sectionStart && i < end) {
                const t = workLines[i].trim();
                const afterKey = t.slice(key.length).trimStart();
                if (t.startsWith(key) && afterKey.startsWith('=')) {
                    if (newLine !== null) {
                        result.push(newLine);
                    }
                    keyFound = true;
                } else {
                    result.push(workLines[i]);
                }
            } else {
                result.push(workLines[i]);
            }
        }

        if (!keyFound && newLine !== null) {
            result.splice(sectionStart + 1, 0, newLine);
        }

        return result.join('\n') + (trailingNewline ? '\n' : '');
    }

    // Section absent.
    if (newLine === null) { return content; }
    let out = content;
    if (out.length > 0 && !out.endsWith('\n')) { out += '\n'; }
    out += `\n${sectionHeader}\n${newLine}\n`;
    return out;
}
