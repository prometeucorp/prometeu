// Tests assert English copy. Node 24 exposes the machine's locale through navigator, and the i18n
// module reads it on import, so pin English before each test file loads it. Tests that need
// another language still stub navigator themselves.
Object.defineProperty(navigator, "language", { value: "en-US", configurable: true });
Object.defineProperty(navigator, "languages", { value: ["en-US"], configurable: true });
