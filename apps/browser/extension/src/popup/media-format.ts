export interface MediaFormat {
  format_id: string;
  label: string;
  ext: string;
  video_codec?: string | null;
  audio_codec?: string | null;
}

export function isVideoOnlyFormat(format: MediaFormat): boolean {
  return typeof format.video_codec === "string"
    && format.video_codec.length > 0
    && !format.audio_codec;
}

export function formatIdForDownload(format: MediaFormat): string {
  return isVideoOnlyFormat(format)
    ? `${format.format_id}+bestaudio/${format.format_id}`
    : format.format_id;
}
