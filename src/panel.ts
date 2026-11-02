import { mount } from "svelte";
import Panel from "./Panel.svelte";
import "./app.css";

const app = mount(Panel, { target: document.getElementById("app")! });
export default app;
