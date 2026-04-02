// chrome.app evasion
// Provides a realistic window.chrome.app mock — headless Chrome may lack it.
(function () {
    const utils = window._pup_utils;
    if (!utils) return;

    if (!window.chrome) {
        Object.defineProperty(window, "chrome", {
            value: {},
            writable: true,
            configurable: true,
        });
    }

    const chrome = window.chrome;

    // chrome.app
    if (!chrome.app) {
        const app = {
            isInstalled: false,
            InstallState: {
                DISABLED: "disabled",
                INSTALLED: "installed",
                NOT_INSTALLED: "not_installed",
            },
            RunningState: {
                CANNOT_RUN: "cannot_run",
                READY_TO_RUN: "ready_to_run",
                RUNNING: "running",
            },
        };
        app.getDetails = utils.makeNativeToString(function getDetails() {
            return null;
        }, "getDetails");
        app.getIsInstalled = utils.makeNativeToString(function getIsInstalled() {
            return false;
        }, "getIsInstalled");
        app.runningState = utils.makeNativeToString(function runningState() {
            return "cannot_run";
        }, "runningState");
        chrome.app = app;
    }

    // chrome.csi
    if (!chrome.csi) {
        chrome.csi = utils.makeNativeToString(function csi() {
            return {
                onloadT: Date.now(),
                startE: Date.now(),
                pageT: performance.now(),
                tran: 15,
            };
        }, "csi");
    }

    // chrome.loadTimes
    if (!chrome.loadTimes) {
        chrome.loadTimes = utils.makeNativeToString(function loadTimes() {
            const now = Date.now() / 1000;
            return {
                commitLoadTime: now,
                connectionInfo: "h2",
                finishDocumentLoadTime: now,
                finishLoadTime: now,
                firstPaintAfterLoadTime: 0,
                firstPaintTime: now,
                navigationType: "Other",
                npnNegotiatedProtocol: "h2",
                requestTime: now - 0.16,
                startLoadTime: now - 0.16,
                wasAlternateProtocolAvailable: false,
                wasFetchedViaSpdy: true,
                wasNpnNegotiated: true,
            };
        }, "loadTimes");
    }

    // chrome.runtime — minimal stub so runtime exists with sendMessage and connect
    if (!chrome.runtime) {
        chrome.runtime = {
            OnInstalledReason: {
                CHROME_UPDATE: "chrome_update",
                INSTALL: "install",
                SHARED_MODULE_UPDATE: "shared_module_update",
                UPDATE: "update",
            },
            OnRestartRequiredReason: {
                APP_UPDATE: "app_update",
                OS_UPDATE: "os_update",
                PERIODIC: "periodic",
            },
            PlatformArch: {
                ARM: "arm",
                MIPS: "mips",
                MIPS64: "mips64",
                X86_32: "x86-32",
                X86_64: "x86-64",
            },
            PlatformNaclArch: {
                ARM: "arm",
                MIPS: "mips",
                MIPS64: "mips64",
                X86_32: "x86-32",
                X86_64: "x86-64",
            },
            PlatformOs: {
                ANDROID: "android",
                CROS: "cros",
                LINUX: "linux",
                MAC: "mac",
                OPENBSD: "openbsd",
                WIN: "win",
            },
            RequestUpdateCheckStatus: {
                NO_UPDATE: "no_update",
                THROTTLED: "throttled",
                UPDATE_AVAILABLE: "update_available",
            },
        };

        chrome.runtime.connect = utils.makeNativeToString(function connect() {
            // Trigger an error as if no extension is connected (normal behavior)
            return { onDisconnect: { addListener: function () { } }, onMessage: { addListener: function () { } }, postMessage: function () { } };
        }, "connect");

        chrome.runtime.sendMessage = utils.makeNativeToString(function sendMessage() {
            // No-op — no extension to receive messages
        }, "sendMessage");
    }
})();
