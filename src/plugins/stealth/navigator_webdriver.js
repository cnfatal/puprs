// navigator.webdriver evasion.
//
// Strategy (mirrors puppeteer-extra-plugin-stealth):
//  - Primary defense is the launch flag `--disable-blink-features=AutomationControlled`
//    which prevents Chromium from setting `navigator.webdriver = true` in the first place.
//  - On modern Chromium this results in `navigator.webdriver === false` (Chrome 89+)
//    or `undefined` (older), both of which are the native state — no patching needed.
//  - Only on very old Chromium (where `webdriver` is still `true`) do we fall back
//    to `delete Object.getPrototypeOf(navigator).webdriver`, which removes the
//    own-accessor cleanly without introducing a detectable replacement getter.
//
// Note: explicitly redefining `webdriver` via `defineProperty` is AVOIDED because
// it leaves a fingerprint in `Object.getOwnPropertyDescriptor(Navigator.prototype, 'webdriver')`.
(function () {
    if (navigator.webdriver === false) {
        // Modern Chromium with the flag applied — already correct.
        return;
    }
    if (navigator.webdriver === undefined) {
        // Pre-89 Chromium without webdriver exposure — already correct.
        return;
    }
    // Old Chromium exposing webdriver === true; remove the prototype accessor.
    try {
        delete Object.getPrototypeOf(navigator).webdriver;
    } catch (_) {
        // Ignore — nothing more we can do without leaving a fingerprint.
    }
})();
