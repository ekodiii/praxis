# Praxis

**Learn programming by building real projects.**

I find the most frustating aspect of the current state of coding education is that in the pursuit of fairness, we have locked the core aspect of creativty away to write rigid tests and projects and requirements. I believe this does work to unlock the *PRACTICE* of coding, and learning syntax and the general understanding of the logic behind the machine is important.

But learning how to truly build something is hard. To step from "Hello World" to making apps is such a crazy transition, one that can only be bridged with practical experience (hence Praxis). I firmly believe learning to code in a browser is capital S STUPID. Learning should be done in the environment one works, with the tools they work with. It's why internships exist, it's why on-the-job training exists, it's there are college students I know with perfect As in every coding class but still can't make a single thing.

Thus the app was born. Locally run tests, with the freedom to develop setup and test code however you please. A platform for 

> This project is currently in **alpha**. I'm working on it don't worry.

---

## Install

1. Download the latest `.dmg` from [Releases](https://github.com/ekodiii/praxis/releases)
2. Open the `.dmg` and drag Praxis to Applications

**macOS security warning:** Because the app is not yet notarized, macOS may say it is "damaged". Run this after installing:

```
xattr -cr /Applications/Praxis.app
```

---

## Requirements

- macOS (Apple Silicon) *I said I'm working on it*
- Whatever you write code on (VSCode, Jetbrains, Xcode, Vim, notepad, anything really)

---

## Tech stack

- [Tauri v2](https://v2.tauri.app/) — desktop shell (Electron pls die)
- Rust — backend logic, subprocess management, test runner
- Vanilla HTML / CSS / JS — no framework, no build step, easier debugging.

---

## Status

- [x] Course UI (chapter list, markdown content, syntax highlighting)
- [x] Automated test runner
- [x] HTTP client panel (mini Postman I guess)
- [x] Progress tracking
- [x] Environment detection (venv / pyenv)
- [ ] Windows support
- [ ] More testing methods (Frontend course soon??)
- [ ] More courses

---

## License
Copyright (c) 2025 Emmanuel Kodamanchilly. All rights reserved.
