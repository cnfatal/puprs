// sourceurl evasion
// Strips __puppeteer_evaluation_script__ and __chromium_evaluation_script__
// sourceURL markers from Error stack traces.
// Detection scripts check stack traces for these known markers.
(function () {
    const _Error = utils.cache.Error;

    // Patch Error.prepareStackTrace if V8 engine
    if (typeof _Error.prepareStackTrace === "undefined") {
        // Install default-like prepareStackTrace that strips sourceURLs
    }

    // Override Error stack property to filter sourceURL markers
    const sourceUrlPattern =
        /__puppeteer_evaluation_script__|__chromium_evaluation_script__/;

    const originalStackDesc = Object.getOwnPropertyDescriptor(
        _Error.prototype,
        "stack"
    );

    if (originalStackDesc && originalStackDesc.get) {
        const origGet = originalStackDesc.get;
        Object.defineProperty(_Error.prototype, "stack", {
            get: function () {
                const stack = origGet.call(this);
                if (typeof stack === "string" && sourceUrlPattern.test(stack)) {
                    return stack
                        .split("\n")
                        .filter((line) => !sourceUrlPattern.test(line))
                        .join("\n");
                }
                return stack;
            },
            set: originalStackDesc.set,
            configurable: true,
        });
    }
})();
