import { mount } from "svelte";

import App from "./App.svelte";
import "./app.css";
import { initPlatform } from "./lib/design/platform";

const target = document.getElementById("app");
if (!target) {
  throw new Error("missing #app mount point");
}

initPlatform();

// The design gallery is a development aid: `?gallery` in the URL, dev server or mock builds only.
// In a normal production build both conditions are statically false, so the dynamic import and
// the whole dev/ folder are removed from the bundle.
const galleryAllowed = import.meta.env.DEV || import.meta.env.VITE_IPC === "mock";
const wantsGallery = galleryAllowed && new URLSearchParams(window.location.search).has("gallery");

async function start(): Promise<unknown> {
  if (wantsGallery) {
    const { default: Gallery } = await import("./dev/Gallery.svelte");
    return mount(Gallery, { target: target as HTMLElement });
  }
  return mount(App, { target: target as HTMLElement });
}

export default start();
