import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { createBrowserRouter, RouterProvider } from "react-router-dom";

import { Shell } from "./components/Shell";
import { Connect } from "./routes/Connect";
import { Vehicle } from "./routes/Vehicle";
import { Effects } from "./routes/Effects";
import { Editor } from "./routes/Editor";
import { Lab } from "./routes/Lab";
import { Safety } from "./routes/Safety";
import { applyTheme, getStoredTheme } from "./theme";
import "./styles.css";

applyTheme(getStoredTheme());

const router = createBrowserRouter([
  {
    path: "/",
    element: <Shell />,
    children: [
      { index: true, element: <Connect /> },
      { path: "vehicle", element: <Vehicle /> },
      { path: "effects", element: <Effects /> },
      { path: "editor", element: <Editor /> },
      { path: "lab", element: <Lab /> },
      { path: "safety", element: <Safety /> },
    ],
  },
]);

const container = document.getElementById("root");
if (!container) throw new Error("missing #root element");

createRoot(container).render(
  <StrictMode>
    <RouterProvider router={router} />
  </StrictMode>,
);
