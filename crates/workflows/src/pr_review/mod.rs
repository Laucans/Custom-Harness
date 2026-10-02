//! La revue consultative d'une PR : second avis en contexte neuf, puis des
//! notes pour l'humain qui doit décider s'il fait confiance au lot.
//!
//! Déclenchée par un hook sur `gh pr create`, pas par un round de la boucle —
//! voir `docs/CUTOVER.md` pour ce qui reste humain dans ce déclenchement.
//!
//! **La surface de design, et la seule** : `stages::table`. Le reste porte ce
//! qui l'entoure — le précontrôle dans `run`, les quatre règles de saut dans
//! `skip_rules`, le texte publié dans `notes`.

pub mod gates;
pub mod notes;
pub mod publish;
pub mod run;
pub mod skip_rules;
pub mod stages;
pub mod state;
