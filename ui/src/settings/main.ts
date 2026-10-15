import { mount } from "svelte";
import "../lib/theme.css";
import Settings from "./Settings.svelte";

export default mount(Settings, { target: document.getElementById("app")! });
