// navigator.languages evasion
// Override navigator.languages to return realistic values.
(function () {

    utils.overridePropertyGetter(
        Object.getPrototypeOf(navigator),
        "languages",
        function () {
            return ["en-US", "en"];
        }
    );
})();
