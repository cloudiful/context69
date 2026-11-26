import { useForm, useStore, type ReactFormExtendedApi } from "@tanstack/react-form";
import { useMemo } from "react";

import {
  createDoclingDraft,
  createSearchDraft,
  type DraftDoclingSettings,
  type DraftSearchSettings,
} from "@/utils/settings";

import type { SettingsSaveSnapshot } from "./settings-save";
import type { AppearanceValues } from "./use-appearance-draft";

/**
 * The native form state the settings provider owns.
 *
 * The Docling and the Search drafts live in one native form, so the shared
 * Save has a single submit entry and every section binds the same state.
 */
export interface SettingsFormValues {
  doclingSettings: DraftDoclingSettings;
  searchSettings: DraftSearchSettings;
  /** The rerank credential the operator typed but never committed. */
  searchRerankApiKey: string;
}

/** The resource one native field change belongs to. */
export type SettingsFormResource = "docling" | "search";

/**
 * The metadata one native submit carries.
 *
 * The click-time snapshot travels with the submit instead of being read from
 * the live form afterwards, so a later edit can never change a dispatched
 * body. The confirmation flag is the vector-space acknowledgement the
 * compatibility dialog collected.
 */
export interface SettingsSubmitMeta {
  readonly confirmVectorSpace: boolean;
  readonly snapshot: SettingsSaveSnapshot;
  /** The appearance captured when Save was clicked; null when it had no changes. */
  readonly appearance: AppearanceValues | null;
  /**
   * The appearance draft as it was when Save was clicked.
   *
   * The baseline is captured even when the appearance had no changes, so a
   * later edit of an initially clean appearance is recognized as a pending
   * edit instead of being committed silently.
   */
  readonly appearanceAtClick: AppearanceValues;
  /** Whether the browser could not store the appearance before this save. */
  readonly appearanceUnresolved: boolean;
}

/** The native form the settings provider owns. */
export type SettingsFormApi = ReactFormExtendedApi<
  SettingsFormValues,
  undefined,
  undefined,
  undefined,
  undefined,
  undefined,
  undefined,
  undefined,
  undefined,
  undefined,
  undefined,
  SettingsSubmitMeta
>;

/**
 * Creates the native settings form.
 *
 * The form lives at the provider's lifetime, so the drafts of every routed
 * section survive a section change. The form-level change listener is the
 * one place a resource revision moves: an edit of any Docling or Search
 * field marks that resource as edited.
 */
export function useSettingsForm(options: {
  onSubmit: (meta: SettingsSubmitMeta) => void | Promise<void>;
  onResourceChange: (resource: SettingsFormResource) => void;
}): SettingsFormApi {
  // The initial values are captured once: a fresh default value on every
  // render would look like a changed default to the form.
  const defaultValues = useMemo<SettingsFormValues>(() => ({
    doclingSettings: createDoclingDraft(),
    searchSettings: createSearchDraft(),
    searchRerankApiKey: "",
  }), []);

  return useForm<
    SettingsFormValues,
    undefined,
    undefined,
    undefined,
    undefined,
    undefined,
    undefined,
    undefined,
    undefined,
    undefined,
    undefined,
    SettingsSubmitMeta
  >({
    defaultValues,
    onSubmit: ({ meta }) => options.onSubmit(meta),
    listeners: {
      onChange: ({ fieldApi }) => {
        const name = String(fieldApi.name);
        // The rerank credential belongs to the Search resource as well, so
        // every Search field moves that one revision.
        if (name.startsWith("docling")) {
          options.onResourceChange("docling");
          return;
        }
        if (name.startsWith("search")) {
          options.onResourceChange("search");
        }
      },
    },
  });
}

/**
 * Subscribes to the native form values.
 *
 * The provider reads the drafts through this subscription, so a changed
 * field re-renders the page that owns the shared Save.
 */
export function useSettingsFormValues(form: SettingsFormApi): SettingsFormValues {
  return useStore(form.store, (state) => state.values);
}
