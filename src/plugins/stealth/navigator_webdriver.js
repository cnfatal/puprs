// navigator.webdriver evasion
// Deletes the webdriver property from Navigator.prototype so detection scripts
// cannot find it. Uses defineProperty with native-looking getter.
(function () {
    const utils = window._pup_utils;
    if (!utils) return;

    // Delete from prototype first (chromium sets it there)
    if ("webdriver" in navigator) {
        delete Object.getPrototypeOf(navigator).webdriver;
    }

    // Then override with a getter that returns undefined
    utils.overridePropertyGetter(
        Object.getPrototypeOf(navigator),
        "webdriver",
        function () {
            return undefined;
        }
    );
})();
