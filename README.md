# Space Peace: Not at War

Eine schlanke, moderne Engine in Rust, die **Star Wars: Empire at War – Forces of Corruption** mit der Mod
**Empire at War Remake 4.0** (Galaxie-Eroberung, Wirtschaft, Raumschlachten – kein Bodenkampf) spielbar macht:
flüssig, in 4K, auf allen CPU-Kernen.

> **Keine Spieldateien in diesem Repository.** Die Engine liest Modelle, Texturen, Musik und Daten zur Laufzeit
> aus deiner eigenen, gekauften Installation (Steam: *Star Wars Empire at War: Gold Pack*) und der abonnierten
> Workshop-Mod. Es werden keine Assets weitergegeben.

## Voraussetzungen

- Windows 10/11 (x64)
- *Star Wars Empire at War: Gold Pack* über Steam
- Workshop-Mod *Empire at War Remake* (ID `1770851727`) abonniert

## Bauen

Doppelklick auf **`build.bat`**. Das Skript installiert bei Bedarf Rust und die MSVC Build Tools (einmalig,
Admin-Bestätigung nötig) und baut das Spiel. Alternativ:

```powershell
.\build.ps1          # Release
.\build.ps1 -Debug   # Debug
```

Es gibt bewusst keine vorgebauten Binaries und keine CI – jeder baut lokal.

## Aufbau

| Crate | Aufgabe |
|---|---|
| `crates/alamo-fs` | MEG-Archive, geschichtetes Dateisystem (Mod → FoC → EaW), Erkennung der Installation |
| `crates/alamo-data` | Spieldaten: XML (Galaxie, Planeten, Handelsrouten, Fraktionen), `.dat`-Texte |

## Status

Frühe Entwicklung. Erster Meilenstein: Galaxiekarte von Remake 4.0 mit Original-Musik.
