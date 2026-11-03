; The browser add-on itself is loaded through each browser's supported UI until
; FetchX has official store listings. Do not register a local folder as an
; externally installed extension; browsers reject that installation method.

!macro RemoveNativeHost Browser
  DeleteRegKey HKCU "Software\${Browser}\NativeMessagingHosts\com.fetchx.native_host"
!macroend

!macro RemoveLegacyChromiumExtension Browser
  DeleteRegKey HKCU "Software\${Browser}\Extensions\iemejekblepcfhklcadedfdmgnhjifio"
!macroend

!macro customInstall
  DetailPrint "Cleaning up legacy FetchX browser extension registrations..."
  SetShellVarContext current

  ; Remove broken local-folder extension registrations from older FetchX installers.
  !insertmacro RemoveLegacyChromiumExtension "Google\Chrome"
  !insertmacro RemoveLegacyChromiumExtension "Google\Chrome Beta"
  !insertmacro RemoveLegacyChromiumExtension "Google\Chrome Dev"
  !insertmacro RemoveLegacyChromiumExtension "Google\Chrome SxS"
  !insertmacro RemoveLegacyChromiumExtension "Chromium"
  !insertmacro RemoveLegacyChromiumExtension "Microsoft\Edge"
  !insertmacro RemoveLegacyChromiumExtension "Microsoft\Edge Beta"
  !insertmacro RemoveLegacyChromiumExtension "Microsoft\Edge Dev"
  !insertmacro RemoveLegacyChromiumExtension "Microsoft\Edge Canary"
  !insertmacro RemoveLegacyChromiumExtension "BraveSoftware\Brave-Browser"
  !insertmacro RemoveLegacyChromiumExtension "BraveSoftware\Brave-Browser-Beta"
  !insertmacro RemoveLegacyChromiumExtension "BraveSoftware\Brave-Browser-Nightly"
  !insertmacro RemoveLegacyChromiumExtension "Vivaldi"
  !insertmacro RemoveLegacyChromiumExtension "Opera Software\Opera Stable"
  !insertmacro RemoveLegacyChromiumExtension "Opera Software\Opera GX Stable"
  !insertmacro RemoveLegacyChromiumExtension "Opera Software\Opera Next"
  !insertmacro RemoveLegacyChromiumExtension "Opera Software\Opera Developer"
  !insertmacro RemoveLegacyChromiumExtension "The Browser Company\Arc"
  !insertmacro RemoveLegacyChromiumExtension "Yandex\YandexBrowser"
  !insertmacro RemoveLegacyChromiumExtension "Thorium"
  DeleteRegValue HKCU "Software\Mozilla\Firefox\Extensions" "FetchX@FetchX.app"
  DeleteRegValue HKCU "Software\LibreWolf\Extensions" "FetchX@FetchX.app"
  DeleteRegValue HKCU "Software\Waterfox\Extensions" "FetchX@FetchX.app"
  DeleteRegValue HKCU "Software\Floorp\Extensions" "FetchX@FetchX.app"
!macroend

!macro customUnInstall
  DetailPrint "Removing FetchX Native Messaging registrations..."
  SetShellVarContext current

  !insertmacro RemoveNativeHost "Google\Chrome"
  !insertmacro RemoveNativeHost "Google\Chrome Beta"
  !insertmacro RemoveNativeHost "Google\Chrome Dev"
  !insertmacro RemoveNativeHost "Google\Chrome SxS"
  !insertmacro RemoveNativeHost "Chromium"
  !insertmacro RemoveNativeHost "Microsoft\Edge"
  !insertmacro RemoveNativeHost "Microsoft\Edge Beta"
  !insertmacro RemoveNativeHost "Microsoft\Edge Dev"
  !insertmacro RemoveNativeHost "Microsoft\Edge Canary"
  !insertmacro RemoveNativeHost "BraveSoftware\Brave-Browser"
  !insertmacro RemoveNativeHost "BraveSoftware\Brave-Browser-Beta"
  !insertmacro RemoveNativeHost "BraveSoftware\Brave-Browser-Nightly"
  !insertmacro RemoveNativeHost "Vivaldi"
  !insertmacro RemoveNativeHost "Opera Software\Opera Stable"
  !insertmacro RemoveNativeHost "Opera Software\Opera GX Stable"
  !insertmacro RemoveNativeHost "Opera Software\Opera Next"
  !insertmacro RemoveNativeHost "Opera Software\Opera Developer"
  !insertmacro RemoveNativeHost "The Browser Company\Arc"
  !insertmacro RemoveNativeHost "Yandex\YandexBrowser"
  !insertmacro RemoveNativeHost "Thorium"
  !insertmacro RemoveNativeHost "Mozilla"
  !insertmacro RemoveNativeHost "LibreWolf"
  !insertmacro RemoveNativeHost "Waterfox"
  !insertmacro RemoveNativeHost "Floorp"
!macroend
