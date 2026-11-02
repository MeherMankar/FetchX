; FetchX NSIS Installer Hook
; Automatically registers the FetchX browser extension and Native Messaging Host
; into all available Chromium and Gecko browsers on Windows.

!macro customInstall
  DetailPrint "Configuring FetchX Browser Integration for all available browsers..."
  SetShellVarContext current

  ; Chromium extension ID: iemejekblepcfhklcadedfdmgnhjifio
  ; Firefox extension ID: FetchX@FetchX.app
  ; Native host name: com.fetchx.native_host

  ; Paths to extension folders and manifests
  StrCpy $0 "$LOCALAPPDATA\com.fetchx.desktop\browser-extension\chromium"
  StrCpy $1 "$LOCALAPPDATA\com.fetchx.desktop\browser-extension\firefox"
  StrCpy $2 "$LOCALAPPDATA\com.fetchx.desktop\native-host\com.fetchx.native_host.json"
  StrCpy $3 "$LOCALAPPDATA\com.fetchx.desktop\native-host\com.fetchx.native_host.firefox.json"

  ; --- Google Chrome Family ---
  WriteRegStr HKCU "Software\Google\Chrome\NativeMessagingHosts\com.fetchx.native_host" "" "$2"
  WriteRegStr HKCU "Software\Google\Chrome\Extensions\iemejekblepcfhklcadedfdmgnhjifio" "path" "$0"
  WriteRegStr HKCU "Software\Google\Chrome\Extensions\iemejekblepcfhklcadedfdmgnhjifio" "version" "0.2.0"

  WriteRegStr HKCU "Software\Google\Chrome Beta\NativeMessagingHosts\com.fetchx.native_host" "" "$2"
  WriteRegStr HKCU "Software\Google\Chrome Beta\Extensions\iemejekblepcfhklcadedfdmgnhjifio" "path" "$0"
  WriteRegStr HKCU "Software\Google\Chrome Beta\Extensions\iemejekblepcfhklcadedfdmgnhjifio" "version" "0.2.0"

  WriteRegStr HKCU "Software\Google\Chrome Dev\NativeMessagingHosts\com.fetchx.native_host" "" "$2"
  WriteRegStr HKCU "Software\Google\Chrome Dev\Extensions\iemejekblepcfhklcadedfdmgnhjifio" "path" "$0"
  WriteRegStr HKCU "Software\Google\Chrome Dev\Extensions\iemejekblepcfhklcadedfdmgnhjifio" "version" "0.2.0"

  WriteRegStr HKCU "Software\Google\Chrome SxS\NativeMessagingHosts\com.fetchx.native_host" "" "$2"
  WriteRegStr HKCU "Software\Google\Chrome SxS\Extensions\iemejekblepcfhklcadedfdmgnhjifio" "path" "$0"
  WriteRegStr HKCU "Software\Google\Chrome SxS\Extensions\iemejekblepcfhklcadedfdmgnhjifio" "version" "0.2.0"

  WriteRegStr HKCU "Software\Chromium\NativeMessagingHosts\com.fetchx.native_host" "" "$2"
  WriteRegStr HKCU "Software\Chromium\Extensions\iemejekblepcfhklcadedfdmgnhjifio" "path" "$0"
  WriteRegStr HKCU "Software\Chromium\Extensions\iemejekblepcfhklcadedfdmgnhjifio" "version" "0.2.0"

  ; --- Microsoft Edge Family ---
  WriteRegStr HKCU "Software\Microsoft\Edge\NativeMessagingHosts\com.fetchx.native_host" "" "$2"
  WriteRegStr HKCU "Software\Microsoft\Edge\Extensions\iemejekblepcfhklcadedfdmgnhjifio" "path" "$0"
  WriteRegStr HKCU "Software\Microsoft\Edge\Extensions\iemejekblepcfhklcadedfdmgnhjifio" "version" "0.2.0"

  WriteRegStr HKCU "Software\Microsoft\Edge Beta\NativeMessagingHosts\com.fetchx.native_host" "" "$2"
  WriteRegStr HKCU "Software\Microsoft\Edge Beta\Extensions\iemejekblepcfhklcadedfdmgnhjifio" "path" "$0"
  WriteRegStr HKCU "Software\Microsoft\Edge Beta\Extensions\iemejekblepcfhklcadedfdmgnhjifio" "version" "0.2.0"

  WriteRegStr HKCU "Software\Microsoft\Edge Dev\NativeMessagingHosts\com.fetchx.native_host" "" "$2"
  WriteRegStr HKCU "Software\Microsoft\Edge Dev\Extensions\iemejekblepcfhklcadedfdmgnhjifio" "path" "$0"
  WriteRegStr HKCU "Software\Microsoft\Edge Dev\Extensions\iemejekblepcfhklcadedfdmgnhjifio" "version" "0.2.0"

  WriteRegStr HKCU "Software\Microsoft\Edge Canary\NativeMessagingHosts\com.fetchx.native_host" "" "$2"
  WriteRegStr HKCU "Software\Microsoft\Edge Canary\Extensions\iemejekblepcfhklcadedfdmgnhjifio" "path" "$0"
  WriteRegStr HKCU "Software\Microsoft\Edge Canary\Extensions\iemejekblepcfhklcadedfdmgnhjifio" "version" "0.2.0"

  ; --- Brave Browser Family ---
  WriteRegStr HKCU "Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\com.fetchx.native_host" "" "$2"
  WriteRegStr HKCU "Software\BraveSoftware\Brave-Browser\Extensions\iemejekblepcfhklcadedfdmgnhjifio" "path" "$0"
  WriteRegStr HKCU "Software\BraveSoftware\Brave-Browser\Extensions\iemejekblepcfhklcadedfdmgnhjifio" "version" "0.2.0"

  WriteRegStr HKCU "Software\BraveSoftware\Brave-Browser-Beta\NativeMessagingHosts\com.fetchx.native_host" "" "$2"
  WriteRegStr HKCU "Software\BraveSoftware\Brave-Browser-Beta\Extensions\iemejekblepcfhklcadedfdmgnhjifio" "path" "$0"
  WriteRegStr HKCU "Software\BraveSoftware\Brave-Browser-Beta\Extensions\iemejekblepcfhklcadedfdmgnhjifio" "version" "0.2.0"

  WriteRegStr HKCU "Software\BraveSoftware\Brave-Browser-Nightly\NativeMessagingHosts\com.fetchx.native_host" "" "$2"
  WriteRegStr HKCU "Software\BraveSoftware\Brave-Browser-Nightly\Extensions\iemejekblepcfhklcadedfdmgnhjifio" "path" "$0"
  WriteRegStr HKCU "Software\BraveSoftware\Brave-Browser-Nightly\Extensions\iemejekblepcfhklcadedfdmgnhjifio" "version" "0.2.0"

  ; --- Vivaldi ---
  WriteRegStr HKCU "Software\Vivaldi\NativeMessagingHosts\com.fetchx.native_host" "" "$2"
  WriteRegStr HKCU "Software\Vivaldi\Extensions\iemejekblepcfhklcadedfdmgnhjifio" "path" "$0"
  WriteRegStr HKCU "Software\Vivaldi\Extensions\iemejekblepcfhklcadedfdmgnhjifio" "version" "0.2.0"

  ; --- Opera Family ---
  WriteRegStr HKCU "Software\Opera Software\Opera Stable\NativeMessagingHosts\com.fetchx.native_host" "" "$2"
  WriteRegStr HKCU "Software\Opera Software\Opera Stable\Extensions\iemejekblepcfhklcadedfdmgnhjifio" "path" "$0"
  WriteRegStr HKCU "Software\Opera Software\Opera Stable\Extensions\iemejekblepcfhklcadedfdmgnhjifio" "version" "0.2.0"

  WriteRegStr HKCU "Software\Opera Software\Opera GX Stable\NativeMessagingHosts\com.fetchx.native_host" "" "$2"
  WriteRegStr HKCU "Software\Opera Software\Opera GX Stable\Extensions\iemejekblepcfhklcadedfdmgnhjifio" "path" "$0"
  WriteRegStr HKCU "Software\Opera Software\Opera GX Stable\Extensions\iemejekblepcfhklcadedfdmgnhjifio" "version" "0.2.0"

  WriteRegStr HKCU "Software\Opera Software\Opera Next\NativeMessagingHosts\com.fetchx.native_host" "" "$2"
  WriteRegStr HKCU "Software\Opera Software\Opera Next\Extensions\iemejekblepcfhklcadedfdmgnhjifio" "path" "$0"
  WriteRegStr HKCU "Software\Opera Software\Opera Next\Extensions\iemejekblepcfhklcadedfdmgnhjifio" "version" "0.2.0"

  WriteRegStr HKCU "Software\Opera Software\Opera Developer\NativeMessagingHosts\com.fetchx.native_host" "" "$2"
  WriteRegStr HKCU "Software\Opera Software\Opera Developer\Extensions\iemejekblepcfhklcadedfdmgnhjifio" "path" "$0"
  WriteRegStr HKCU "Software\Opera Software\Opera Developer\Extensions\iemejekblepcfhklcadedfdmgnhjifio" "version" "0.2.0"

  ; --- Arc, Yandex, Thorium ---
  WriteRegStr HKCU "Software\The Browser Company\Arc\NativeMessagingHosts\com.fetchx.native_host" "" "$2"
  WriteRegStr HKCU "Software\The Browser Company\Arc\Extensions\iemejekblepcfhklcadedfdmgnhjifio" "path" "$0"
  WriteRegStr HKCU "Software\The Browser Company\Arc\Extensions\iemejekblepcfhklcadedfdmgnhjifio" "version" "0.2.0"

  WriteRegStr HKCU "Software\Yandex\YandexBrowser\NativeMessagingHosts\com.fetchx.native_host" "" "$2"
  WriteRegStr HKCU "Software\Yandex\YandexBrowser\Extensions\iemejekblepcfhklcadedfdmgnhjifio" "path" "$0"
  WriteRegStr HKCU "Software\Yandex\YandexBrowser\Extensions\iemejekblepcfhklcadedfdmgnhjifio" "version" "0.2.0"

  WriteRegStr HKCU "Software\Thorium\NativeMessagingHosts\com.fetchx.native_host" "" "$2"
  WriteRegStr HKCU "Software\Thorium\Extensions\iemejekblepcfhklcadedfdmgnhjifio" "path" "$0"
  WriteRegStr HKCU "Software\Thorium\Extensions\iemejekblepcfhklcadedfdmgnhjifio" "version" "0.2.0"

  ; --- Mozilla Firefox & Gecko Family ---
  WriteRegStr HKCU "Software\Mozilla\NativeMessagingHosts\com.fetchx.native_host" "" "$3"
  WriteRegStr HKCU "Software\Mozilla\Firefox\Extensions" "FetchX@FetchX.app" "$1"

  WriteRegStr HKCU "Software\LibreWolf\NativeMessagingHosts\com.fetchx.native_host" "" "$3"
  WriteRegStr HKCU "Software\LibreWolf\Extensions" "FetchX@FetchX.app" "$1"

  WriteRegStr HKCU "Software\Waterfox\NativeMessagingHosts\com.fetchx.native_host" "" "$3"
  WriteRegStr HKCU "Software\Waterfox\Extensions" "FetchX@FetchX.app" "$1"

  WriteRegStr HKCU "Software\Floorp\NativeMessagingHosts\com.fetchx.native_host" "" "$3"
  WriteRegStr HKCU "Software\Floorp\Extensions" "FetchX@FetchX.app" "$1"
!macroend

!macro customUnInstall
  DetailPrint "Removing FetchX Browser Integration for all browsers..."
  SetShellVarContext current

  ; Google Chrome
  DeleteRegKey HKCU "Software\Google\Chrome\NativeMessagingHosts\com.fetchx.native_host"
  DeleteRegKey HKCU "Software\Google\Chrome\Extensions\iemejekblepcfhklcadedfdmgnhjifio"
  DeleteRegKey HKCU "Software\Google\Chrome Beta\NativeMessagingHosts\com.fetchx.native_host"
  DeleteRegKey HKCU "Software\Google\Chrome Beta\Extensions\iemejekblepcfhklcadedfdmgnhjifio"
  DeleteRegKey HKCU "Software\Google\Chrome Dev\NativeMessagingHosts\com.fetchx.native_host"
  DeleteRegKey HKCU "Software\Google\Chrome Dev\Extensions\iemejekblepcfhklcadedfdmgnhjifio"
  DeleteRegKey HKCU "Software\Google\Chrome SxS\NativeMessagingHosts\com.fetchx.native_host"
  DeleteRegKey HKCU "Software\Google\Chrome SxS\Extensions\iemejekblepcfhklcadedfdmgnhjifio"
  DeleteRegKey HKCU "Software\Chromium\NativeMessagingHosts\com.fetchx.native_host"
  DeleteRegKey HKCU "Software\Chromium\Extensions\iemejekblepcfhklcadedfdmgnhjifio"

  ; Microsoft Edge
  DeleteRegKey HKCU "Software\Microsoft\Edge\NativeMessagingHosts\com.fetchx.native_host"
  DeleteRegKey HKCU "Software\Microsoft\Edge\Extensions\iemejekblepcfhklcadedfdmgnhjifio"
  DeleteRegKey HKCU "Software\Microsoft\Edge Beta\NativeMessagingHosts\com.fetchx.native_host"
  DeleteRegKey HKCU "Software\Microsoft\Edge Beta\Extensions\iemejekblepcfhklcadedfdmgnhjifio"
  DeleteRegKey HKCU "Software\Microsoft\Edge Dev\NativeMessagingHosts\com.fetchx.native_host"
  DeleteRegKey HKCU "Software\Microsoft\Edge Dev\Extensions\iemejekblepcfhklcadedfdmgnhjifio"
  DeleteRegKey HKCU "Software\Microsoft\Edge Canary\NativeMessagingHosts\com.fetchx.native_host"
  DeleteRegKey HKCU "Software\Microsoft\Edge Canary\Extensions\iemejekblepcfhklcadedfdmgnhjifio"

  ; Brave
  DeleteRegKey HKCU "Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\com.fetchx.native_host"
  DeleteRegKey HKCU "Software\BraveSoftware\Brave-Browser\Extensions\iemejekblepcfhklcadedfdmgnhjifio"
  DeleteRegKey HKCU "Software\BraveSoftware\Brave-Browser-Beta\NativeMessagingHosts\com.fetchx.native_host"
  DeleteRegKey HKCU "Software\BraveSoftware\Brave-Browser-Beta\Extensions\iemejekblepcfhklcadedfdmgnhjifio"
  DeleteRegKey HKCU "Software\BraveSoftware\Brave-Browser-Nightly\NativeMessagingHosts\com.fetchx.native_host"
  DeleteRegKey HKCU "Software\BraveSoftware\Brave-Browser-Nightly\Extensions\iemejekblepcfhklcadedfdmgnhjifio"

  ; Vivaldi
  DeleteRegKey HKCU "Software\Vivaldi\NativeMessagingHosts\com.fetchx.native_host"
  DeleteRegKey HKCU "Software\Vivaldi\Extensions\iemejekblepcfhklcadedfdmgnhjifio"

  ; Opera
  DeleteRegKey HKCU "Software\Opera Software\Opera Stable\NativeMessagingHosts\com.fetchx.native_host"
  DeleteRegKey HKCU "Software\Opera Software\Opera Stable\Extensions\iemejekblepcfhklcadedfdmgnhjifio"
  DeleteRegKey HKCU "Software\Opera Software\Opera GX Stable\NativeMessagingHosts\com.fetchx.native_host"
  DeleteRegKey HKCU "Software\Opera Software\Opera GX Stable\Extensions\iemejekblepcfhklcadedfdmgnhjifio"
  DeleteRegKey HKCU "Software\Opera Software\Opera Next\NativeMessagingHosts\com.fetchx.native_host"
  DeleteRegKey HKCU "Software\Opera Software\Opera Next\Extensions\iemejekblepcfhklcadedfdmgnhjifio"
  DeleteRegKey HKCU "Software\Opera Software\Opera Developer\NativeMessagingHosts\com.fetchx.native_host"
  DeleteRegKey HKCU "Software\Opera Software\Opera Developer\Extensions\iemejekblepcfhklcadedfdmgnhjifio"

  ; Arc, Yandex, Thorium
  DeleteRegKey HKCU "Software\The Browser Company\Arc\NativeMessagingHosts\com.fetchx.native_host"
  DeleteRegKey HKCU "Software\The Browser Company\Arc\Extensions\iemejekblepcfhklcadedfdmgnhjifio"
  DeleteRegKey HKCU "Software\Yandex\YandexBrowser\NativeMessagingHosts\com.fetchx.native_host"
  DeleteRegKey HKCU "Software\Yandex\YandexBrowser\Extensions\iemejekblepcfhklcadedfdmgnhjifio"
  DeleteRegKey HKCU "Software\Thorium\NativeMessagingHosts\com.fetchx.native_host"
  DeleteRegKey HKCU "Software\Thorium\Extensions\iemejekblepcfhklcadedfdmgnhjifio"

  ; Mozilla Firefox & Gecko Family
  DeleteRegKey HKCU "Software\Mozilla\NativeMessagingHosts\com.fetchx.native_host"
  DeleteRegValue HKCU "Software\Mozilla\Firefox\Extensions" "FetchX@FetchX.app"
  DeleteRegKey HKCU "Software\LibreWolf\NativeMessagingHosts\com.fetchx.native_host"
  DeleteRegValue HKCU "Software\LibreWolf\Extensions" "FetchX@FetchX.app"
  DeleteRegKey HKCU "Software\Waterfox\NativeMessagingHosts\com.fetchx.native_host"
  DeleteRegValue HKCU "Software\Waterfox\Extensions" "FetchX@FetchX.app"
  DeleteRegKey HKCU "Software\Floorp\NativeMessagingHosts\com.fetchx.native_host"
  DeleteRegValue HKCU "Software\Floorp\Extensions" "FetchX@FetchX.app"
!macroend
