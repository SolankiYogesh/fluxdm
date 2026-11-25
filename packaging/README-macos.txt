1. Drag FluxDM into Applications, then open it.
   The app is not notarised by Apple, so macOS will block it the first time.
   Open System Settings > Privacy & Security and choose "Open Anyway".

2. For browser capture, open Terminal and run:
   /Applications/FluxDM.app/Contents/MacOS/fluxdm-host install

3. In your browser open chrome://extensions, turn on Developer mode,
   choose "Load unpacked", press Cmd+Shift+G and paste:
   /Applications/FluxDM.app/Contents/Resources/extension
