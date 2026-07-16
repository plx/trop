import favicon from "@trop/design-system/assets/favicon.svg?raw";
import type { APIRoute } from "astro";

export const prerender = true;

export const GET: APIRoute = () =>
  new Response(favicon, {
    headers: {
      "Content-Type": "image/svg+xml",
    },
  });
