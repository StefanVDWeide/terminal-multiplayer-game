class GameClient:
    def __init__(self) -> None:
        self.hp = 10

    def attack(self, attack_power: int):
        self.hp = self.hp - attack_power
