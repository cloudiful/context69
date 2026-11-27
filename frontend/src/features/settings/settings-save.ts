import type {
  DoclingSettingsResponse,
  SearchSettingsResponse,
  TranslationProviderInput,
} from "@/services/api";
import {
  doclingResponseToDraft,
  searchResponseToPayload,
  type DraftDoclingSettings,
  type DraftRuntimeSettings,
  type DraftSearchSettings,
} from "@/utils/settings";

import type { AppearanceValues } from "./use-appearance-draft";

/** One translation provider as the settings page holds it. */
export interface TranslationProviderDraft extends Omit<TranslationProviderInput, "enabled"> {
  enabled: boolean;
  has_api_key: boolean;
  current_month_characters: number;
}

/** Whether two appearance drafts describe the same choice. */
export function sameAppearance(left: AppearanceValues, right: AppearanceValues) {
  return left.locale === right.locale && left.theme === right.theme && left.timeZone === right.timeZone;
}

/** The revision each resource was at when a snapshot was taken. */
export interface SettingsSaveRevisions {
  readonly runtime: number;
  readonly docling: number;
  readonly search: number;
  readonly translation: number;
}

/** The resources one save applies to. */
export interface SettingsSaveApplicable {
  readonly runtime: boolean;
  readonly docling: boolean;
  readonly search: boolean;
  readonly translation: boolean;
}

/**
 * The click-time snapshot of every resource a save may dispatch.
 *
 * The snapshot is taken when the operator clicks Save, before any request is
 * awaited, so a later edit can neither change a dispatched body nor be
 * overwritten by a late response.
 */
export interface SettingsSaveSnapshot {
  readonly runtime: DraftRuntimeSettings;
  readonly docling: DraftDoclingSettings;
  readonly search: DraftSearchSettings;
  /** The rerank credential the operator typed but never committed. */
  readonly searchRerankApiKey: string;
  readonly translation: readonly TranslationProviderDraft[];
  /** The revision each resource was at when the snapshot was taken. */
  readonly revisions: SettingsSaveRevisions;
  /** The resources the snapshot applies to. */
  readonly applicable: SettingsSaveApplicable;
}

/** The values one snapshot is captured from. */
export interface SettingsSaveCapture {
  readonly runtime: DraftRuntimeSettings;
  readonly docling: DraftDoclingSettings;
  readonly search: DraftSearchSettings;
  readonly searchRerankApiKey: string;
  readonly translation: readonly TranslationProviderDraft[];
  readonly revisions: SettingsSaveRevisions;
  readonly applicable: SettingsSaveApplicable;
}

/**
 * Copies a draft so the dispatched body cannot follow a later edit.
 *
 * The drafts are plain data, so a JSON round trip is the copy: it keeps the
 * snapshot independent from the live form state.
 */
function copySettingsValue<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T;
}

/**
 * Captures the click-time snapshot of every applicable resource.
 *
 * The capture is deliberately synchronous: the caller reads the live drafts
 * and the live revisions in one step, so a dispatched body and the revision
 * it belongs to always describe the same moment.
 */
export function captureSettingsSnapshot(capture: SettingsSaveCapture): SettingsSaveSnapshot {
  return {
    runtime: copySettingsValue(capture.runtime),
    docling: copySettingsValue(capture.docling),
    search: copySettingsValue(capture.search),
    searchRerankApiKey: capture.searchRerankApiKey,
    translation: capture.translation.map((provider) => ({ ...provider })),
    revisions: { ...capture.revisions },
    applicable: { ...capture.applicable },
  };
}

/**
 * The visible state an adopted Docling commit leaves behind.
 *
 * A response never carries the stored credential, so the adopted draft holds
 * an empty key: the secret is redacted by the adopted commit itself, never by
 * a failed request and never by a response that lost the race with a newer
 * edit.
 */
export function adoptDoclingCommit(response: DoclingSettingsResponse): DraftDoclingSettings {
  return doclingResponseToDraft(response);
}

/**
 * The visible state an adopted Search commit leaves behind.
 *
 * The committed rerank credential is the stored one, so the browser keeps no
 * copy of the key that was submitted.
 */
export function adoptSearchCommit(response: SearchSettingsResponse): {
  search: DraftSearchSettings;
  searchRerankApiKey: string;
} {
  return { search: searchResponseToPayload(response), searchRerankApiKey: "" };
}

/**
 * One resource of the shared save.
 *
 * A peer owns three decisions: it dispatches the captured payload, it knows
 * whether the visible draft still matches the revision that payload was
 * captured at, and it adopts a committed response into the visible draft.
 */
export interface SettingsSavePeer {
  /** The name one failure is reported under. */
  readonly key: string;
  /** Dispatches the captured payload and keeps the service response. */
  readonly save: () => Promise<void>;
  /** Adopts the committed response into the visible draft. */
  readonly adopt: () => void;
  /** Whether the visible draft still matches the captured revision. */
  readonly isCurrent: () => boolean;
}

/**
 * Builds a peer from a typed dispatch and a typed adoption.
 *
 * The response stays inside the peer, so the shared save never has to know
 * the response type of a resource.
 */
export function defineSettingsSavePeer<TResponse>(peer: {
  readonly key: string;
  readonly save: () => Promise<TResponse>;
  readonly adopt: (response: TResponse) => void;
  readonly isCurrent: () => boolean;
}): SettingsSavePeer {
  let response: TResponse | undefined;
  return {
    key: peer.key,
    save: async () => {
      response = await peer.save();
    },
    adopt: () => {
      if (response !== undefined) {
        peer.adopt(response);
      }
    },
    isCurrent: peer.isCurrent,
  };
}

/** The outcome of one peer of the shared save. */
export interface SettingsSavePeerOutcome {
  readonly key: string;
  readonly status: "fulfilled" | "rejected";
  /** Whether the committed response was adopted into the visible draft. */
  readonly adopted: boolean;
}

/** One rejected request of the shared save. */
export interface SettingsSaveFailure {
  readonly key: string;
  readonly reason: unknown;
}

/** The outcome of the shared save. */
export interface SettingsSaveOutcome {
  /** Whether the admission step rejected; no peer was dispatched then. */
  readonly admissionFailed: boolean;
  readonly peers: readonly SettingsSavePeerOutcome[];
  readonly failures: readonly SettingsSaveFailure[];
}

/**
 * The state one completed save is assessed against.
 *
 * The baseline is the state the save started from: the revisions and the
 * appearance captured when the operator clicked Save. The current state is
 * what the page holds once the requests settled.
 */
export interface SettingsSaveCompletion {
  /** The revisions captured when the save was clicked. */
  readonly baseline: SettingsSaveRevisions;
  /** The revisions of the page once the save completed. */
  readonly current: SettingsSaveRevisions;
  /** The appearance draft captured when the save was clicked. */
  readonly appearanceBaseline: AppearanceValues;
  /** The live appearance draft once the save completed. */
  readonly appearance: AppearanceValues;
}

/**
 * Whether an unsaved edit remains after a save completed.
 *
 * Every resource counts, not only the ones this save submitted: a resource
 * that was clean when the operator clicked Save can be edited while the save
 * is running, and that newer edit is still unsaved. The appearance counts the
 * same way: the live draft is compared with the captured one, so a later edit
 * of an initially clean appearance keeps the page pending.
 */
export function settingsSaveHasPendingEdit(completion: SettingsSaveCompletion): boolean {
  const { baseline, current } = completion;
  return current.runtime !== baseline.runtime
    || current.docling !== baseline.docling
    || current.search !== baseline.search
    || current.translation !== baseline.translation
    || !sameAppearance(completion.appearance, completion.appearanceBaseline);
}

/**
 * Runs the shared save lifecycle.
 *
 * The admission step stays authoritative: its rejection stops every peer and
 * the appearance commit, so a refused runtime never leaves a half-written
 * page. The peers keep the settled semantics: every fulfilled peer updates
 * its cache and is adopted, even when another peer failed, and a peer whose
 * visible draft moved on since the capture keeps that newer edit instead of
 * adopting a response that no longer describes the form.
 */
export async function runSettingsSave(input: {
  readonly admission: SettingsSavePeer | null;
  readonly peers: readonly SettingsSavePeer[];
}): Promise<SettingsSaveOutcome> {
  if (input.admission) {
    try {
      await input.admission.save();
    } catch (reason) {
      return {
        admissionFailed: true,
        peers: [],
        failures: [{ key: input.admission.key, reason }],
      };
    }
    if (input.admission.isCurrent()) {
      input.admission.adopt();
    }
  }

  const results = await Promise.allSettled(input.peers.map((peer) => peer.save()));
  const peers: SettingsSavePeerOutcome[] = [];
  const failures: SettingsSaveFailure[] = [];

  results.forEach((result, index) => {
    const peer = input.peers[index];
    if (result.status === "fulfilled") {
      const adopted = peer.isCurrent();
      if (adopted) {
        peer.adopt();
      }
      peers.push({ key: peer.key, status: "fulfilled", adopted });
      return;
    }
    peers.push({ key: peer.key, status: "rejected", adopted: false });
    failures.push({ key: peer.key, reason: result.reason });
  });

  return { admissionFailed: false, peers, failures };
}
