import type { RemoteNativeMagnetMetadata } from "./protocol";

export type TorrentSelectionState =
  | { kind: "loading" }
  | { kind: "unavailable"; message: string }
  | { kind: "acquiring"; metadata: RemoteNativeMagnetMetadata }
  | {
    kind: "ready";
    metadata: RemoteNativeMagnetMetadata;
    selectedFileIndexes: number[];
    error: string | null;
  }
  | {
    kind: "confirming";
    metadata: RemoteNativeMagnetMetadata;
    selectedFileIndexes: number[];
  }
  | { kind: "confirmed"; metadata: RemoteNativeMagnetMetadata }
  | { kind: "failed"; message: string; stale: boolean };

export type TorrentSelectionAction =
  | { type: "loading" }
  | { type: "unavailable"; message: string }
  | { type: "metadata"; metadata: RemoteNativeMagnetMetadata }
  | { type: "selection"; indexes: number[] }
  | { type: "confirming" }
  | { type: "confirmed" }
  | { type: "failed"; message: string; stale?: boolean };

export type TorrentEngineChoice = "automatic" | "aria2" | "native_torrent";

export function supportsNativeMagnetSelection(capabilities: Iterable<string>): boolean {
  const advertised = new Set(capabilities);
  return advertised.has("download.torrentMetadata")
    && advertised.has("download.selectTorrentFiles");
}

export function defaultTorrentEngineChoice(capabilities: Iterable<string>): TorrentEngineChoice {
  return supportsNativeMagnetSelection(capabilities) ? "native_torrent" : "automatic";
}

export const initialTorrentSelectionState: TorrentSelectionState = { kind: "loading" };

export function reduceTorrentSelection(
  state: TorrentSelectionState,
  action: TorrentSelectionAction,
): TorrentSelectionState {
  switch (action.type) {
    case "loading":
      return { kind: "loading" };
    case "unavailable":
      return { kind: "unavailable", message: action.message };
    case "metadata":
      if (action.metadata.metadata_state !== "ready") {
        return { kind: "acquiring", metadata: action.metadata };
      }
      if (action.metadata.selection_confirmed) {
        return { kind: "confirmed", metadata: action.metadata };
      }
      if (state.kind === "ready" || state.kind === "confirming") {
        return {
          kind: "ready",
          metadata: action.metadata,
          selectedFileIndexes: state.selectedFileIndexes.filter((index) =>
            action.metadata.files.some((file) => file.index === index)
          ),
          error: null,
        };
      }
      return {
        kind: "ready",
        metadata: action.metadata,
        selectedFileIndexes: action.metadata.selected_file_indexes,
        error: null,
      };
    case "selection":
      return state.kind === "ready"
        ? { ...state, selectedFileIndexes: action.indexes, error: null }
        : state;
    case "confirming":
      return state.kind === "ready"
        ? { ...state, kind: "confirming" }
        : state;
    case "confirmed":
      return state.kind === "confirming"
        ? { kind: "confirmed", metadata: { ...state.metadata, selection_confirmed: true } }
        : state;
    case "failed":
      return { kind: "failed", message: action.message, stale: action.stale ?? false };
    default:
      return state;
  }
}
