//! La revue consultative d'une PR : second avis en contexte neuf, puis des
//! notes pour l'humain qui doit décider s'il fait confiance au lot.
//!
//! Déclenchée par un hook sur `gh pr create`, pas par un round de la boucle —
//! voir `docs/CUTOVER.md` pour ce qui reste humain dans ce déclenchement.
