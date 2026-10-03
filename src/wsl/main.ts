import { TauriSession } from "./tauri";
import { mount } from "./view";
const runtime = new TauriSession();
mount(document.querySelector<HTMLElement>("#app")!, runtime, runtime, runtime);
