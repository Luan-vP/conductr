import * as vscode from 'vscode';
import { SafetyFaderPanel } from './safetyFaderPanel';

export function activate(context: vscode.ExtensionContext): void {
    const provider = new SafetyFaderPanel(context);

    context.subscriptions.push(
        vscode.window.registerWebviewViewProvider(
            SafetyFaderPanel.viewType,
            provider,
            { webviewOptions: { retainContextWhenHidden: true } },
        ),
    );

    context.subscriptions.push(
        vscode.commands.registerCommand('conductr.openSafetyFader', () => {
            vscode.commands.executeCommand('workbench.view.extension.conductr');
        }),
    );
}

export function deactivate(): void {}
