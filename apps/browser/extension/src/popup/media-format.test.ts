import { describe, expect, it } from "vitest";

import { formatIdForDownload, isVideoOnlyFormat } from "./media-format";

describe("media format download selector", () => {
  it("adds best audio to video-only formats with a fallback to video", () => {
    const format = {
      format_id: "137",
      label: "1920x1080",
      ext: "mp4",
      video_codec: "avc1.640028",
      audio_codec: null,
    };

    expect(isVideoOnlyFormat(format)).toBe(true);
    expect(formatIdForDownload(format)).toBe("137+bestaudio/137");
  });

  it("keeps formats with audio or unknown codecs unchanged", () => {
    expect(formatIdForDownload({
      format_id: "18",
      label: "360p",
      ext: "mp4",
      video_codec: "avc1",
      audio_codec: "mp4a",
    })).toBe("18");

    expect(formatIdForDownload({
      format_id: "140",
      label: "audio only",
      ext: "m4a",
      audio_codec: "mp4a",
    })).toBe("140");

    expect(formatIdForDownload({
      format_id: "unknown",
      label: "unknown",
      ext: "mp4",
    })).toBe("unknown");
  });
});
