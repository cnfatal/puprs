(() => {
    // ---- Deferred ----
    class Deferred {
        #resolve;
        #reject;
        #promise;
        #finished = false;
        constructor() {
            this.#promise = new Promise((resolve, reject) => {
                this.#resolve = resolve;
                this.#reject = reject;
            });
        }
        resolve(value) {
            if (this.#finished) return;
            this.#finished = true;
            this.#resolve(value);
        }
        reject(reason) {
            if (this.#finished) return;
            this.#finished = true;
            this.#reject(reason);
        }
        finished() {
            return this.#finished;
        }
        valueOrThrow() {
            return this.#promise;
        }
    }

    // ---- MutationPoller ----
    class MutationPoller {
        #fn;
        #root;
        #observer;
        #deferred;
        constructor(fn, root) {
            this.#fn = fn;
            this.#root = root;
        }
        async start() {
            const deferred = (this.#deferred = new Deferred());
            const result = await this.#fn();
            if (result) {
                deferred.resolve(result);
                return;
            }
            this.#observer = new MutationObserver(async () => {
                const result = await this.#fn();
                if (!result) return;
                deferred.resolve(result);
                await this.stop();
            });
            this.#observer.observe(this.#root, {
                childList: true,
                subtree: true,
                attributes: true,
            });
        }
        async stop() {
            if (this.#deferred && !this.#deferred.finished()) {
                this.#deferred.reject(new Error("Polling stopped"));
            }
            if (this.#observer) {
                this.#observer.disconnect();
                this.#observer = undefined;
            }
        }
        result() {
            return this.#deferred.valueOrThrow();
        }
    }

    // ---- RAFPoller ----
    class RAFPoller {
        #fn;
        #deferred;
        constructor(fn) {
            this.#fn = fn;
        }
        async start() {
            const deferred = (this.#deferred = new Deferred());
            const result = await this.#fn();
            if (result) {
                deferred.resolve(result);
                return;
            }
            const poll = async () => {
                if (deferred.finished()) return;
                const result = await this.#fn();
                if (!result) {
                    window.requestAnimationFrame(poll);
                    return;
                }
                deferred.resolve(result);
                await this.stop();
            };
            window.requestAnimationFrame(poll);
        }
        async stop() {
            if (this.#deferred && !this.#deferred.finished()) {
                this.#deferred.reject(new Error("Polling stopped"));
            }
        }
        result() {
            return this.#deferred.valueOrThrow();
        }
    }

    // ---- IntervalPoller ----
    class IntervalPoller {
        #fn;
        #ms;
        #interval;
        #deferred;
        constructor(fn, ms) {
            this.#fn = fn;
            this.#ms = ms;
        }
        async start() {
            const deferred = (this.#deferred = new Deferred());
            const result = await this.#fn();
            if (result) {
                deferred.resolve(result);
                return;
            }
            this.#interval = setInterval(async () => {
                const result = await this.#fn();
                if (!result) return;
                deferred.resolve(result);
                await this.stop();
            }, this.#ms);
        }
        async stop() {
            if (this.#deferred && !this.#deferred.finished()) {
                this.#deferred.reject(new Error("Polling stopped"));
            }
            if (this.#interval) {
                clearInterval(this.#interval);
                this.#interval = undefined;
            }
        }
        result() {
            return this.#deferred.valueOrThrow();
        }
    }

    // ---- createFunction ----
    const createFunction = (() => {
        const cache = new Map();
        return (fnStr) => {
            let fn = cache.get(fnStr);
            if (fn) return fn;
            fn = new Function("return " + fnStr)();
            cache.set(fnStr, fn);
            return fn;
        };
    })();

    // ---- checkVisibility ----
    // Matches puppeteer's util.ts implementation:
    // visibility style + bounding box check (no el.checkVisibility API dependency).
    const HIDDEN_VISIBILITY_VALUES = ["hidden", "collapse"];

    function isBoundingBoxEmpty(element) {
        const rect = element.getBoundingClientRect();
        return rect.width === 0 || rect.height === 0;
    }

    function checkVisibility(node, visible) {
        if (!node) {
            return visible === false;
        }
        if (visible === undefined || visible === null) {
            return node;
        }
        const element =
            node.nodeType === Node.TEXT_NODE ? node.parentElement : node;
        const style = window.getComputedStyle(element);
        const isVisible =
            style &&
            !HIDDEN_VISIBILITY_VALUES.includes(style.visibility) &&
            !isBoundingBoxEmpty(element);
        return visible === isVisible ? node : false;
    }

    return Object.freeze({
        Deferred,
        MutationPoller,
        RAFPoller,
        IntervalPoller,
        createFunction,
        checkVisibility,
    });
})()
