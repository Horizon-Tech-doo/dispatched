import { lazy, Suspense } from "react";
import LegacyApp from "./LegacyApp";

const catalogueAllowed = import.meta.env.DEV || import.meta.env.VITE_DISPATCHED_MOCK === "1";
const Catalogue = catalogueAllowed ? lazy(() => import("./dev/Catalogue")) : null;

export default function App() {
  if (Catalogue && window.location.hash === "#catalogue") {
    return (
      <Suspense fallback={null}>
        <Catalogue />
      </Suspense>
    );
  }
  return <LegacyApp />;
}
