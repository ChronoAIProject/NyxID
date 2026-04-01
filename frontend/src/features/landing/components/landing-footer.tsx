export function LandingFooter() {
  return (
    <footer className="border-t border-landing-border-subtle px-6 py-6">
      <p className="text-center font-mono text-xs text-gray-500">
        &copy; {new Date().getFullYear()} Chrono AI. All rights reserved.
      </p>
    </footer>
  );
}
