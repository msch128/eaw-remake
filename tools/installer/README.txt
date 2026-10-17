EAW:Remake
==========

An unofficial, open-source Rust/Bevy reimplementation of Galactic Conquest and space
battles for Star Wars: Empire at War - Forces of Corruption, playing the Empire at War
Remake 4.0 mod. This setup contains no game files.

What you need from Steam
------------------------
1. Star Wars: Empire at War - Gold Pack (includes Forces of Corruption), Steam app 32470
   https://store.steampowered.com/app/32470/
2. The mod Empire at War Remake 4.0, subscribed on the Steam Workshop (id 1770851727),
   with its download finished
   https://steamcommunity.com/sharedfiles/filedetails/?id=1770851727

EAW:Remake finds both by itself through Steam, also in other Steam library folders.

Start
-----
Start menu, EAW Remake. To see what was found, run in a terminal:
  eaw-remake.exe --check-install > check.txt
(the game opens no console window, so its output only shows when redirected).
Exit code 0 means both were found, 2 the game is missing, 3 the mod is missing.

Game or mod in a custom place
-----------------------------
  eaw-remake.exe --game-dir "D:\Games\Star Wars Empire at War" --mod-dir "D:\Mods\1770851727"
or set the environment variables EAW_GAME_DIR and EAW_MOD_DIR.

Saves, settings and logs
------------------------
They are in %APPDATA%\SpacePeace (settings.toml, saves, logs). Uninstalling EAW:Remake
keeps that folder; delete it by hand to remove your saves.
If something goes wrong, attach the newest file of %APPDATA%\SpacePeace\logs to your
bug report: https://github.com/msch128/eaw-remake/issues

License
-------
MIT, see LICENSE. Credits and thanks: CREDITS.md.
EAW:Remake is not affiliated with Lucasfilm, Disney, Petroglyph Games, Valve or the
Empire at War Remake team. Star Wars is a trademark of Lucasfilm Ltd.
Source code: https://github.com/msch128/eaw-remake
