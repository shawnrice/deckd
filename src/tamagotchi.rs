use std::sync::{Arc, Mutex};
use std::time::Instant;

/// A pixel pet that lives on the LCD strip.
/// It reacts to your work habits — happy when you ship PRs,
/// hungry when you have pending reviews, sleepy during meetings.
#[derive(Debug, Clone)]
pub struct Pet {
    pub name: String,
    pub species: Species,
    pub mood: Mood,
    pub energy: u8,      // 0-100
    pub happiness: u8,   // 0-100
    pub hunger: u8,      // 0-100 (100 = starving)
    pub xp: u32,
    pub level: u32,
    pub action: Action,  // current idle animation
    frame: u8,
    action_started: Instant,
    action_duration_secs: u32,
    last_update: Instant,
    last_fed: Instant,
    last_pet: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Species {
    Cat,
    Dog,
    Penguin,
    Ghost,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mood {
    Happy,
    Neutral,
    Sad,
    Sleeping,
    Excited,
    Hungry,
    Coding,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    Idle,
    Walking,
    Dancing,
    Eating,
    Napping,
    Celebrating,
    LookingAround,
    Typing,
}

impl Species {
    /// Get the ASCII art for this species in a given mood and animation frame
    pub fn sprite(&self, mood: &Mood, action: &Action, frame: u8) -> &'static str {
        let f = frame % 4;
        match self {
            Species::Cat => match action {
                Action::Walking => match f {
                    0 => "=^.^=  ",
                    1 => " =^.^= ",
                    2 => "  =^.^=",
                    _ => " =^.^= ",
                },
                Action::Dancing => match f {
                    0 => "~=^.^=~",
                    1 => "\\=^o^=/",
                    2 => "~=^.^=~",
                    _ => "/=^o^=\\",
                },
                Action::Napping => match f % 2 {
                    0 => "=^-.-^= z",
                    _ => "=^-.-^=zZ",
                },
                Action::Eating => match f % 2 {
                    0 => "=^.^= ~@",
                    _ => "=^o^=  @",
                },
                Action::Celebrating => match f % 2 {
                    0 => "\\=^★^=/",
                    _ => " =^✧^= ",
                },
                Action::LookingAround => match f {
                    0 => "=^.^=  ?",
                    1 => "=^.^= ??",
                    2 => "=^o^=  !",
                    _ => "=^.^=   ",
                },
                Action::Typing => match f % 2 {
                    0 => "=^.^=⌨ ",
                    _ => "=^-^=⌨ ",
                },
                Action::Idle => match mood {
                    Mood::Happy => match f % 2 { 0 => "=^.^=", _ => "=^‿^=" },
                    Mood::Excited => match f % 2 { 0 => "=^★^=", _ => "=^✧^=" },
                    Mood::Sad => match f % 2 { 0 => "=;.;=", _ => "=T.T=" },
                    Mood::Hungry => match f % 2 { 0 => "=>.<=", _ => "=°△°=" },
                    Mood::Sleeping => match f % 2 { 0 => "=^-^=zZ", _ => "=^-^=zzZ" },
                    Mood::Coding => match f % 2 { 0 => "=^.^=⌨", _ => "=^-^=⌨" },
                    Mood::Neutral => match f % 2 { 0 => "=^.^=", _ => "=^_^=" },
                },
            },
            Species::Dog => match action {
                Action::Walking => match f {
                    0 => "U^ᴥ^U  ",
                    1 => " U^ᴥ^U ",
                    2 => "  U^ᴥ^U",
                    _ => " U^ᴥ^U ",
                },
                Action::Dancing => match f % 2 {
                    0 => "∪^ᴥ^∪♪",
                    _ => "∪°ᴥ°∪♫",
                },
                Action::Napping => match f % 2 {
                    0 => "U-ᴥ-U z",
                    _ => "U-ᴥ-UzZ",
                },
                Action::Eating => match f % 2 {
                    0 => "U^ᴥ^U♡",
                    _ => "U°ᴥ°U~",
                },
                Action::Celebrating => match f % 2 {
                    0 => "U★ᴥ★U!",
                    _ => "U✧ᴥ✧U!",
                },
                Action::LookingAround => match f {
                    0 => "U^ᴥ^U ?",
                    1 => "U^ᴥ^U??",
                    _ => "U^ᴥ^U  ",
                },
                Action::Typing => match f % 2 {
                    0 => "U^ᴥ^U⌨",
                    _ => "U-ᴥ-U⌨",
                },
                Action::Idle => match mood {
                    Mood::Happy => match f % 2 { 0 => "U^ᴥ^U", _ => "U°ᴥ°U" },
                    Mood::Excited => match f % 2 { 0 => "U★ᴥ★U", _ => "U✧ᴥ✧U" },
                    Mood::Sad => match f % 2 { 0 => "U;ᴥ;U", _ => "UTᴥTU" },
                    Mood::Hungry => "U>ᴥ<U",
                    Mood::Sleeping => match f % 2 { 0 => "U-ᴥ-UzZ", _ => "U-ᴥ-Uzz" },
                    Mood::Coding => match f % 2 { 0 => "U^ᴥ^U⌨", _ => "U-ᴥ-U⌨" },
                    Mood::Neutral => match f % 2 { 0 => "U^ᴥ^U", _ => "U·ᴥ·U" },
                },
            },
            Species::Penguin => match action {
                Action::Walking => match f {
                    0 => "(·◇·)  ",
                    1 => " (·◇·) ",
                    2 => "  (·◇·)",
                    _ => " (·◇·) ",
                },
                Action::Dancing => match f % 2 {
                    0 => "♪(·◇·)♪",
                    _ => "♫(°◇°)♫",
                },
                Action::Napping => match f % 2 {
                    0 => "(-◇-)zZ",
                    _ => "(-◇-)zzZ",
                },
                Action::Eating => match f % 2 {
                    0 => "(·◇·)~🐟",
                    _ => "(°◇°) 🐟",
                },
                Action::Celebrating => match f % 2 {
                    0 => "(★◇★)!!",
                    _ => "(✧◇✧)!",
                },
                Action::LookingAround => match f {
                    0 => "(·◇·) ?",
                    1 => "(·◇·)  ",
                    2 => " (·◇·)?",
                    _ => "(·◇·)  ",
                },
                Action::Typing => match f % 2 {
                    0 => "(·◇·)⌨",
                    _ => "(-◇-)⌨",
                },
                Action::Idle => match mood {
                    Mood::Happy => match f % 2 { 0 => "(·◇·)", _ => "(°◇°)" },
                    Mood::Excited => match f % 2 { 0 => "(★◇★)", _ => "(✧◇✧)" },
                    Mood::Sad => match f % 2 { 0 => "(;◇;)", _ => "(T◇T)" },
                    Mood::Hungry => "(>◇<)",
                    Mood::Sleeping => match f % 2 { 0 => "(-◇-)zZ", _ => "(-◇-)zzZ" },
                    Mood::Coding => match f % 2 { 0 => "(·◇·)⌨", _ => "(-◇-)⌨" },
                    Mood::Neutral => match f % 2 { 0 => "(·◇·)", _ => "(·_·)" },
                },
            },
            Species::Ghost => match action {
                Action::Walking => match f {
                    0 => "ᗣ   ",
                    1 => " ᗣ  ",
                    2 => "  ᗣ ",
                    _ => " ᗣ  ",
                },
                Action::Dancing => match f % 2 {
                    0 => "~ᗣ~♪",
                    _ => "♫ᗣ~ ",
                },
                Action::Napping => match f % 2 {
                    0 => "ᗣ zZ",
                    _ => "ᗣ zzZ",
                },
                Action::Eating => match f % 2 {
                    0 => "ᗣ ~◆",
                    _ => "ᗣ  ◆",
                },
                Action::Celebrating => match f % 2 {
                    0 => "✧ᗣ✧!",
                    _ => "★ᗣ★!",
                },
                Action::LookingAround => match f {
                    0 => "ᗣ ?  ",
                    1 => "ᗣ ?? ",
                    2 => "ᗣ  ! ",
                    _ => "ᗣ    ",
                },
                Action::Typing => match f % 2 {
                    0 => "ᗣ⌨",
                    _ => "ᗣ⌨ ",
                },
                Action::Idle => match mood {
                    Mood::Happy | Mood::Excited => match f % 2 { 0 => "ᗣ ♡", _ => "ᗣ ♥" },
                    Mood::Sad => "ᗣ ;",
                    Mood::Hungry => "ᗣ ~",
                    Mood::Sleeping => match f % 2 { 0 => "ᗣzZ", _ => "ᗣzzZ" },
                    Mood::Coding => match f % 2 { 0 => "ᗣ⌨", _ => "ᗣ⌨ " },
                    Mood::Neutral => match f % 2 { 0 => "ᗣ  ", _ => " ᗣ " },
                },
            },
        }
    }
}

impl Pet {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            species: Species::Cat,
            mood: Mood::Happy,
            energy: 80,
            happiness: 70,
            hunger: 20,
            xp: 0,
            level: 1,
            action: Action::Idle,
            frame: 0,
            action_started: Instant::now(),
            action_duration_secs: 10,
            last_update: Instant::now(),
            last_fed: Instant::now(),
            last_pet: Instant::now(),
        }
    }

    /// Call every few seconds to update pet state
    pub fn tick(&mut self) {
        let elapsed = self.last_update.elapsed().as_secs();
        if elapsed < 3 {
            return;
        }
        self.last_update = Instant::now();
        self.frame = self.frame.wrapping_add(1);

        // Hunger increases over time
        self.hunger = (self.hunger + 1).min(100);

        // Happiness decays slowly if not interacted with
        if self.last_pet.elapsed().as_secs() > 300 {
            self.happiness = self.happiness.saturating_sub(1);
        }

        // Energy recovers slowly when idle
        if self.mood == Mood::Sleeping {
            self.energy = (self.energy + 2).min(100);
        }

        // Pick random idle actions periodically
        if self.action_started.elapsed().as_secs() >= self.action_duration_secs as u64 {
            self.pick_new_action();
        }

        // Update mood based on stats
        self.mood = if self.energy < 20 {
            Mood::Sleeping
        } else if self.hunger > 80 {
            Mood::Hungry
        } else if self.happiness < 30 {
            Mood::Sad
        } else if self.happiness > 80 {
            Mood::Excited
        } else {
            Mood::Neutral
        };
    }

    fn pick_new_action(&mut self) {
        // Use frame as simple pseudo-random
        let roll = (self.frame.wrapping_mul(7).wrapping_add(13)) % 20;
        self.action = match roll {
            0..=4 => Action::Idle,
            5..=7 => Action::Walking,
            8..=9 => Action::LookingAround,
            10..=11 => Action::Dancing,
            12 => Action::Napping,
            13..=14 => Action::Typing,
            _ => Action::Idle,
        };
        // Override based on mood
        if self.mood == Mood::Sleeping {
            self.action = Action::Napping;
        } else if self.mood == Mood::Hungry {
            self.action = Action::LookingAround;
        }
        self.action_started = Instant::now();
        self.action_duration_secs = match self.action {
            Action::Walking => 8,
            Action::Dancing => 6,
            Action::Napping => 15,
            Action::LookingAround => 5,
            Action::Typing => 10,
            _ => 12,
        };
    }

    /// Feed the pet (call when you merge a PR or close an issue)
    pub fn feed(&mut self) {
        self.hunger = self.hunger.saturating_sub(30);
        self.happiness = (self.happiness + 10).min(100);
        self.xp += 10;
        self.last_fed = Instant::now();
        self.action = Action::Eating;
        self.action_started = Instant::now();
        self.action_duration_secs = 5;
        self.mood = Mood::Happy;
        self.check_level_up();
    }

    /// Pet the pet (call on LCD touch)
    pub fn pet(&mut self) {
        self.happiness = (self.happiness + 5).min(100);
        self.xp += 2;
        self.last_pet = Instant::now();
        self.mood = Mood::Happy;
    }

    /// PR was shipped — big reward
    pub fn ship_pr(&mut self) {
        self.happiness = (self.happiness + 20).min(100);
        self.hunger = self.hunger.saturating_sub(20);
        self.xp += 50;
        self.action = Action::Celebrating;
        self.action_started = Instant::now();
        self.action_duration_secs = 8;
        self.mood = Mood::Excited;
        self.check_level_up();
    }

    /// Review completed — small reward
    #[allow(dead_code)]
    pub fn complete_review(&mut self) {
        self.happiness = (self.happiness + 10).min(100);
        self.xp += 20;
        self.mood = Mood::Happy;
        self.check_level_up();
    }

    /// Pending reviews make pet hungry
    pub fn pending_reviews(&mut self, count: u32) {
        if count > 0 {
            self.hunger = (self.hunger + count as u8 * 2).min(100);
        }
    }

    fn check_level_up(&mut self) {
        let new_level = (self.xp / 100) + 1;
        if new_level > self.level {
            self.level = new_level;
            self.action = Action::Celebrating;
            self.action_started = Instant::now();
            self.action_duration_secs = 10;
            self.mood = Mood::Excited;
            self.happiness = 100;
            // Evolve species at certain levels
            if self.level == 5 {
                self.species = Species::Dog;
            } else if self.level == 10 {
                self.species = Species::Penguin;
            } else if self.level == 20 {
                self.species = Species::Ghost;
            }
        }
    }

    /// Get the sprite for current state
    pub fn sprite(&self) -> &'static str {
        self.species.sprite(&self.mood, &self.action, self.frame)
    }

    /// Get a status line
    pub fn status(&self) -> String {
        let species_name = match self.species {
            Species::Cat => "cat",
            Species::Dog => "dog",
            Species::Penguin => "penguin",
            Species::Ghost => "ghost",
        };
        format!(
            "Lv.{} {} · {}",
            self.level,
            species_name,
            match self.action {
                Action::Idle => match self.mood {
                    Mood::Happy => "vibing",
                    Mood::Excited => "HYPED!",
                    Mood::Sad => "needs love",
                    Mood::Hungry => "reviews pls",
                    Mood::Sleeping => "resting",
                    Mood::Coding => "hacking",
                    Mood::Neutral => "chillin",
                },
                Action::Walking => "exploring",
                Action::Dancing => "grooving",
                Action::Eating => "nom nom",
                Action::Napping => "zzz",
                Action::Celebrating => "woohoo!",
                Action::LookingAround => "curious",
                Action::Typing => "coding",
            }
        )
    }
}

pub type SharedPet = Arc<Mutex<Pet>>;

pub fn new_shared(name: &str) -> SharedPet {
    Arc::new(Mutex::new(Pet::new(name)))
}
