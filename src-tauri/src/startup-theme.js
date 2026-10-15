(() => {
  const preference = __THEME_PREFERENCE__;
  window.__MM_THEME__ = preference;
  const dark = preference === 'dark' || (preference === 'system' && matchMedia('(prefers-color-scheme: dark)').matches);
  const apply = () => {
    const root = document.documentElement;
    if (!root) return false;
    root.dataset.theme = dark ? 'dark' : 'light';
    root.style.colorScheme = dark ? 'dark' : 'light';
    root.style.backgroundColor = dark ? '#171d28' : '#fcfdff';
    return true;
  };
  if (!apply()) {
    const observer = new MutationObserver(() => { if (apply()) observer.disconnect(); });
    observer.observe(document, {childList:true, subtree:true});
  }
})();
