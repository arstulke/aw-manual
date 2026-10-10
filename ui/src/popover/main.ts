import { mount } from "svelte";
import "../lib/theme.css";
import Popover from "./Popover.svelte";

// The popover is a keyboard-driven tool window: no context menu.
document.addEventListener("contextmenu", (e) => e.preventDefault());

export default mount(Popover, { target: document.getElementById("app")! });
