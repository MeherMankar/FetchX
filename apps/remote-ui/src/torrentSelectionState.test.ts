import { describe, expect, it } from "vitest";

import type { RemoteNativeMagnetMetadata } from "./protocol";
import { initialTorrentSelectionState, reduceTorrentSelection } from "./torrentSelectionState";

const pending: RemoteNativeMagnetMetadata = {
  id: "download-id",
  metadata_state: "pending",
  info_hash: "0123456789012345678901234567890123456789",
  name: null,
  files: [],
  selected_file_indexes: [],
  selected_total_bytes: 0,
  selection_confirmed: false,
};

const ready: RemoteNativeMagnetMetadata = {
  ...pending,
  metadata_state: "ready",
  name: "bundle",
  files: [
    { index: 0, path: ["bundle", "one.bin"], size: 10 },
    { index: 1, path: ["bundle", "two.bin"], size: 20 },
  ],
};

describe("native magnet selection UI state", () => {
  it("starts loading and shows metadata-acquisition state", () => {
    expect(initialTorrentSelectionState).toEqual({ kind: "loading" });
    expect(reduceTorrentSelection(initialTorrentSelectionState, {
      type: "metadata",
      metadata: pending,
    })).toEqual({ kind: "acquiring", metadata: pending });
  });

  it("renders a ready selection from the server and tracks confirmation", () => {
    const loaded = reduceTorrentSelection(initialTorrentSelectionState, {
      type: "metadata",
      metadata: { ...ready, selected_file_indexes: [1] },
    });
    expect(loaded).toMatchObject({ kind: "ready", selectedFileIndexes: [1] });
    const selected = reduceTorrentSelection(loaded, { type: "selection", indexes: [0] });
    expect(selected).toMatchObject({ kind: "ready", selectedFileIndexes: [0] });
    const confirming = reduceTorrentSelection(selected, { type: "confirming" });
    expect(confirming).toMatchObject({ kind: "confirming", selectedFileIndexes: [0] });
    expect(reduceTorrentSelection(confirming, { type: "confirmed" })).toMatchObject({
      kind: "confirmed",
      metadata: { selection_confirmed: true },
    });
  });

  it("preserves stale metadata and unavailable-capability errors for the detail view", () => {
    expect(reduceTorrentSelection(initialTorrentSelectionState, {
      type: "failed",
      message: "Metadata changed.",
      stale: true,
    })).toEqual({ kind: "failed", message: "Metadata changed.", stale: true });
    expect(reduceTorrentSelection(initialTorrentSelectionState, {
      type: "unavailable",
      message: "Update the server.",
    })).toEqual({ kind: "unavailable", message: "Update the server." });
  });
});
