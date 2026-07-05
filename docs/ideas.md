# Ideas & Backlog

Loose references and future-direction notes. Not commitments.

## Avatar (face + body for the Waifu)

Options to explore:
- AI generated: AnimateDiff ControlNet Animation v1.0 (ComfyUI) — https://www.youtube.com/watch?v=HbfDjAMFi6w
- 3D: VRM model + Three.js lip-sync (see voice-agent-architecture.md Phase 2+)

VRM model sources:
- BOOTH.pm (booth.pm/en) — Japanese creators, best quality, use BOOTHPLORER (boothplorer.com) for better sorting
- CGTrader — sort by popularity, royalty-free bundles
- vrmodels.store — VRChat-focused, has VRM exports
- premadevtubermodels.com — VTuber-ready with lip sync, $20-24

Must-haves: VRM 1.0, visemes (aa/ih/ou/ee/oh), expression presets, humanoid bones

## Skills (cool skills to have)

- Language teaching: "How to Become Conversational in Any Language - Polyglot Explains" — https://www.youtube.com/watch?v=jI4cIjz7zU8

## Companion / chatbot / AI-dating sites (reference)

- https://rizzai.ai/
- https://datingai.pro/
- https://www.yourmove.ai/
- https://winggg.com/

---

## Original README (migrated from root README.md)

# Personal-Assistant-Waifu
Your very own Personal Assistant Waifu

## Design Decisions

- Bring your own keys (LLM, TTS, STT), this shit is private.
- [ ] Separate the skills into its own repository? (Check ProjectAliceSkills)

## Roadmap

- [ ] Multiplatform AI text chat with one model
- [ ] TTS and STT, call mode and voice message mode
- [ ] 2D Avatar Waifu with lipsync
- [ ] Memory: RAG
- [ ] Memory: Context editor
- [ ] Memory: Dynamic caching of full topics
- [ ] Personality: Select/Edit/Create your personality

- [ ] Game Mode: Play games with your Waifu
  - [ ] Games repo vs using skills repo
  - [ ] Dead by AI game

- [ ] Control browser
  - [ ] Chrome extension vs
  - [ ] Testing framework API

- [ ] Control PC

## Individual Random Specific Features

- [ ] Remind me of my appointments
- [ ] Manage my list of priorities
- [ ] Automate scripts in my computer
- [ ] Answer my business whatsapp
- [ ] Control Claude Code by Voice
- [ ] Skill: Send CVs on Linkedin/custom
- [ ] Skill: Auto follow / like socials github/instagram/tiktok

## References

### Personal assistants

https://github.com/Jackywine/Bella

https://github.com/project-alice-assistant/ProjectAlice
https://github.com/project-alice-assistant/ProjectAliceSkills

https://github.com/leon-ai/leon
https://github.com/leon-ai/leon/tree/develop/skills

https://github.com/DragonComputer/Dragonfire

### Tools to give it Skills

(Convert office docs to Markdown)
https://github.com/microsoft/markitdown

### LLMS

(Check this, apparently is the shit)
https://github.com/MoonshotAI/Kimi-K2

### Games that use AI

https://deathbyai.gg/

### Games SAAS

(Games architecture)
https://joinplayroom.com/
(Strategic games partnership)
https://www.littleumbrella.gg/

## TODO

- [ ] Check Mistral AI STT https://mistral.ai/news/voxtral starts at $0.001 per minute
- [ ] Check Inworld TTS https://inworld.ai/tts $5/million characters

## TO READ

- [ ] https://inworld.ai/case-study/how-inworld-helped-the-ai-game-death-by-ai-with-20-million-players-reach-profitability
- [ ] https://inworld.ai/blog/improved-character-brain
- [ ] https://inworld.ai/blog/introducing-dynamic-relationships
- [ ] https://inworld.ai/blog/introducing-long-term-memory
