// iframe.contentWindow evasion
// Patches the contentWindow property of srcdoc iframes so that detection
// scripts running inside iframes also see consistent navigator properties.
// In headless Chrome, srcdoc iframe contentWindow may leak different values.
(function () {
    const utils = window._pup_utils;
    if (!utils) return;

    // Monitor iframe creation and patch their contentWindow
    try {
        const originalDescriptor = Object.getOwnPropertyDescriptor(
            HTMLIFrameElement.prototype,
            "contentWindow"
        );
        if (!originalDescriptor || !originalDescriptor.get) return;

        const originalGet = originalDescriptor.get;

        const patchedGet = function () {
            const iframe = this;
            const result = originalGet.call(iframe);

            // Only intercept srcdoc and about:blank iframes
            if (!result) return result;
            if (iframe.srcdoc || iframe.src === "about:blank" || !iframe.src) {
                // Proxy the contentWindow so navigator checks pass
                const handler = {
                    get: function (target, prop) {
                        if (prop === "navigator") {
                            // Return the parent navigator (which is already patched)
                            return window.navigator;
                        }
                        if (prop === "chrome") {
                            return window.chrome;
                        }
                        const value = Reflect.get(target, prop);
                        if (typeof value === "function") {
                            return value.bind(target);
                        }
                        return value;
                    },
                };

                try {
                    return new Proxy(result, utils.stripProxyFromErrors(handler));
                } catch {
                    return result;
                }
            }
            return result;
        };

        utils.makeNativeToString(patchedGet, "get contentWindow");

        Object.defineProperty(HTMLIFrameElement.prototype, "contentWindow", {
            get: patchedGet,
            configurable: true,
            enumerable: true,
        });
    } catch (e) {
        // Silently fail if we can't patch
    }
})();
