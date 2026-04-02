// navigator.languages evasion
// Override navigator.languages to return realistic values.
(function () {
    const utils = window._pup_utils;
    if (!utils) return;

    utils.overridePropertyGetter(
        Object.getPrototypeOf(navigator),
        "languages",
        function () {
            return ["en-US", "en"];
        }
    );
})();
