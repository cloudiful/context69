if (!Element.prototype.scrollIntoView) {
  Element.prototype.scrollIntoView = () => undefined;
}

if (!document.queryCommandSupported) {
  document.queryCommandSupported = () => false;
}
