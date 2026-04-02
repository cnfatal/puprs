// navigator.hardwareConcurrency evasion
// Override to return a common value (headless may report abnormal values).
(function () {
    const utils = window._pup_utils;
    if (!utils) return;

    utils.overridePropertyGetter(
        Object.getPrototypeOf(navigator),
        "hardwareConcurrency",
        function () {
            return 4;
        }
    );
})();
