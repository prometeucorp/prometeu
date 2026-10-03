import { MockSession } from "./mock";
import { mount } from "./view";
const runtime = new MockSession();
mount(document.querySelector<HTMLElement>("#app")!, runtime, runtime, runtime);
