// navigator.hardwareConcurrency evasion
// Override to return a common value (headless may report abnormal values).
(function () {

    utils.overridePropertyGetter(
        Object.getPrototypeOf(navigator),
        "hardwareConcurrency",
        function () {
            return 4;
        }
    );
})();
