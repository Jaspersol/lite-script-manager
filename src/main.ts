import { mount } from "svelte";
import App from "./App.svelte";
import "./app.css";
import { initLocale } from "./lib/i18n.svelte";

void initLocale();
const app = mount(App, { target: document.getElementById("app")! });
export default app;
