Welcome to Pokémon Card GB2 native port {tag}! Pick the download for your computer, phone or tablet below.

New here? The [read-me]({readme}) covers the controls, settings, saves and how the port works.

## Installation

You'll need your own dump of the Japanese cartridge, *Pokémon Card GB2 - GR Dan Sanjou!* (2 MiB, SHA-1 `a7e12bcc5f514e3aad8de570fd511aab0a308822`). The downloads contain no game data: the app reads it from your ROM.

- **Windows:** Download the [Windows ZIP]({downloads}windows-x86_64.zip), extract it into a folder you can save files in, put your ROM in its `rom` folder, and open `pokemon-tcg2.exe`. Keep the included DLLs beside it.
- **macOS:** Download the ZIP for [Apple Silicon]({downloads}macos-aarch64.zip) or [Intel]({downloads}macos-x86_64.zip), extract it, put your ROM in its `rom` folder, and open `Pokémon Card GB2.app`. It is ad-hoc signed, not notarized: the first time, right-click it and choose **Open**.
- **Linux:** Download the archive for [most PCs (x86-64)]({downloads}linux-x86_64.tar.gz) or [ARM devices (AArch64)]({downloads}linux-aarch64.tar.gz), extract it into a folder you can save files in, put your ROM in its `rom` folder, and run `pokemon-tcg2`. It runs on any distribution from 2020 on (glibc 2.31) with your system's own graphics drivers.
- **Android:** On your phone or tablet (Android 8 or newer), download the [APK]({downloads}android.apk) and open it to install, allowing your browser or file manager to install apps if Android asks. Start Pokémon Card GB2 and choose your ROM when it asks; the app keeps a copy. The console's buttons are on the screen, and a gamepad or keyboard works too.

The app keeps everything beside itself: battery saves in `saves/` (the same `.sav` format emulators use, so an existing save can be copied in), save states, screenshots and `settings.ini`. On Android they are in the app's folder, `Android/data/io.github.pokemon_card_game_2_native_port/files`, which a computer can open over USB.

### Steam Deck & Steam Machine

Using SteamOS? You can launch the Linux app straight from your Steam library:

1. Switch to Desktop Mode and create a folder for the game inside your home folder.
2. Extract the [Linux x86-64 archive]({downloads}linux-x86_64.tar.gz) into that folder (right-click it and choose **Extract**), and put your ROM in its `rom` folder.
3. In Steam, choose **Games → Add a Non-Steam Game to My Library**, browse to `pokemon-tcg2` in the extracted folder, and add it.
4. Return to Gaming Mode and open the game from your library's **Non-Steam** tab.

### Updating

**Updating an existing installation?** Replace the app and its included files, and keep `rom/`, `saves/` and `settings.ini`. On Android, install the new APK over the old one; uninstalling would delete the app's folder. Battery saves carry over between versions; save states may need the version that created them.

Need a hand? The [installation guide]({guide}) has more setup help and the current limitations.

## What's new

{changelog}
