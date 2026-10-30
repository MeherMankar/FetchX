export function selectBrowserApi<TFirefox, TChromium>(
  firefoxApi: TFirefox | undefined,
  chromiumApi: TChromium,
): TFirefox | TChromium {
  return firefoxApi ?? chromiumApi;
}
