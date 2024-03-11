# Two player online game for in the terminal

Initial implementation based on: <https://github.com/tokio-rs/tokio/blob/master/examples/chat.rs>

## Design decisions to be made

- Where should the core game logic live? On the server or at the player side?
- Use UUID instead of sockerAdr?

## Game loop

A pokemon like game where you fight 1v1 and have multiple "pokemons" which all have their own HP, attack power and defence. The goals is to eliminate all the pokemon of your opponent.
