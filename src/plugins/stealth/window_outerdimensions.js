// window.outerdimensions evasion
// In headless mode, window.outerWidth and outerHeight are 0.
// This patches them to match innerWidth/innerHeight (plus typical chrome UI).
(function () {

    // Only patch if outer dimensions are zero (headless indicator)
    if (window.outerWidth === 0 && window.outerHeight === 0) {
        utils.overridePropertyGetter(window, "outerWidth", function () {
            return window.innerWidth;
        });
        utils.overridePropertyGetter(window, "outerHeight", function () {
            return window.innerHeight + 85; // typical Chrome toolbar height
        });
        utils.overridePropertyGetter(window, "screenX", function () {
            return 13; // typical non-zero position
        });
        utils.overridePropertyGetter(window, "screenY", function () {
            return 25;
        });
    }
})();
