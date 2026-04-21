// navigator.vendor evasion
// Returns "Google Inc." — the standard vendor for Chrome on all platforms.
// Some headless environments may return empty string.
(function () {

    utils.overridePropertyGetter(
        Object.getPrototypeOf(navigator),
        "vendor",
        function () {
            return "Google Inc.";
        }
    );
})();
