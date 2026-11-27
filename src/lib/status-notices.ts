/**
 * The status bar's notices, in the order it shows them, from plain store snapshots. Each has a
 * stable id per source (the list's key), so a changed message updates in place. The
 * component renders them and owns the action controls and their help. Pure, so the order, the
 * copy and the either/or pairs are tested without the UI.
 */
import { engineNotices } from './audio/engine-notices';
import type { MemoPlayerState } from './audio/memo-player';
import type { ControlInfo } from './control/protocol';
import type { PrefsState } from './prefs/store';
import type { SessionState } from './project/session';
import { recordingNotices, type NoticeAction, type RecordingNotice } from './recording/memo-ui';
import type { RecordingUiState } from './recording/types';
import type { RuntimeState } from './strudel/runtime';

export interface StatusNoticesInput {
  runtime: Pick<RuntimeState, 'evalError' | 'outputError' | 'tempoOverride' | 'engineWarnings'>;
  session: Pick<SessionState, 'error' | 'metaError' | 'notice'>;
  prefs: Pick<PrefsState, 'error'>;
  control: ControlInfo;
  /** Null until the recording controller is wired: no take notices. */
  recording: RecordingUiState | null;
  /** Null without memos: no playback notices. */
  player: MemoPlayerState | null;
  /** Absent: `navigator.platform` (see `memo-ui.ts`). */
  platform?: Navigator['platform'];
}

export interface StatusNotice {
  /** Stable per source, so a changed message updates in place. */
  id: string;
  tone: RecordingNotice['tone']; // 'warning' | 'danger'
  text: string;
  /** The tooltip; never set with `action`. */
  title?: string;
  /** The control after the text, rendered with its help by the component. */
  action?: NoticeAction;
}

export function statusNotices(input: StatusNoticesInput): StatusNotice[] {
  const { runtime, session, prefs, control } = input;
  const out: StatusNotice[] = [];
  // An evaluate error hides the output error.
  if (runtime.evalError) {
    out.push({ id: 'eval-error', tone: 'danger', text: `error: ${runtime.evalError}` });
  } else if (runtime.outputError) {
    out.push({ id: 'output-error', tone: 'danger', text: `audio error: ${runtime.outputError}` });
  }
  if (session.error) out.push({ id: 'session-error', tone: 'danger', text: session.error });
  if (session.metaError) {
    out.push({ id: 'meta-error', tone: 'danger', text: `meta: ${session.metaError}` });
  }
  if (prefs.error) out.push({ id: 'prefs-error', tone: 'danger', text: `prefs: ${prefs.error}` });
  if (runtime.tempoOverride) {
    out.push({
      id: 'tempo-override',
      tone: 'warning',
      text: 'setcps in the code overrides meta.bpm',
    });
  }
  for (const n of engineNotices(runtime.engineWarnings)) {
    const notice: StatusNotice = { id: `engine:${n.key}`, tone: 'warning', text: n.text };
    if (n.detail !== undefined) notice.title = n.detail;
    out.push(notice);
  }
  if (session.notice) {
    out.push({ id: 'claude-edit', tone: 'warning', text: `Claude edited ${session.notice.title}` });
  }
  // The socket path is the tooltip, empty when unknown (the attribute is still set).
  if (control.status === 'in-use') {
    out.push({
      id: 'control-in-use',
      tone: 'warning',
      text: 'control socket in use',
      title: control.path ?? '',
    });
  } else if (control.status === 'disabled') {
    // Svelte renders a null as nothing; a template literal would write `null`.
    out.push({
      id: 'control-disabled',
      tone: 'warning',
      text: `control disabled: ${control.detail ?? ''}`,
    });
  }
  out.push(...recordingNotices(input.recording, input.player, input.platform));
  return out;
}
