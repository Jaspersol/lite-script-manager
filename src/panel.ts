import { mount } from "svelte";
import Panel from "./Panel.svelte";
import "./app.css";
import { initLocale } from "./lib/i18n.svelte";

void initLocale();
const app = mount(Panel, { target: document.getElementById("app")! });
export default app;
