// webgl.vendor evasion
// Fakes WebGL renderer and vendor info to avoid headless detection.
// Headless Chrome often reports "Google Inc." / "ANGLE (...)  SwiftShader"
// which is a known headless fingerprint.
(function () {

    const VENDOR = "Intel Inc.";
    const RENDERER = "Intel Iris OpenGL Engine";

    // Override getParameter for both WebGL and WebGL2
    const getParameterProxyHandler = {
        apply: function (target, thisArg, args) {
            const param = args[0];
            const debugExt =
                thisArg.getExtension("WEBGL_debug_renderer_info");
            if (debugExt) {
                if (param === debugExt.UNMASKED_VENDOR_WEBGL) {
                    return VENDOR;
                }
                if (param === debugExt.UNMASKED_RENDERER_WEBGL) {
                    return RENDERER;
                }
            }
            return Reflect.apply(target, thisArg, args);
        },
    };

    // Patch WebGLRenderingContext
    if (typeof WebGLRenderingContext !== "undefined") {
        const origGetParam =
            WebGLRenderingContext.prototype.getParameter;
        const proxy = new Proxy(
            origGetParam,
            utils.stripProxyFromErrors(getParameterProxyHandler)
        );
        utils.makeNativeToString(proxy, "getParameter");
        WebGLRenderingContext.prototype.getParameter = proxy;
    }

    // Patch WebGL2RenderingContext
    if (typeof WebGL2RenderingContext !== "undefined") {
        const origGetParam2 =
            WebGL2RenderingContext.prototype.getParameter;
        const proxy2 = new Proxy(
            origGetParam2,
            utils.stripProxyFromErrors(getParameterProxyHandler)
        );
        utils.makeNativeToString(proxy2, "getParameter");
        WebGL2RenderingContext.prototype.getParameter = proxy2;
    }
})();
