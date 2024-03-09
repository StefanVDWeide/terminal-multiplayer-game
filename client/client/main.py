import socket
from game.game import GameClient

HOST = "127.0.0.1"
PORT = 8080


def send_message(sock, message):
    # Append newline to message, as the Rust server uses LinesCodec
    message_with_newline = message + "\n"
    sock.sendall(message_with_newline.encode())


def receive_message(sock):
    # Receive messages until newline is encountered
    message = ""
    while not message.endswith("\n"):
        chunk = sock.recv(1).decode()
        if not chunk:  # Connection closed
            return None
        message += chunk
    return message.strip()


def main() -> None:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.connect((HOST, PORT))

        # Receive prompt from server and send room name
        print(receive_message(s))  # "Please enter your room name:"
        room_name = input()
        send_message(s, room_name)

        # Receive next prompt and send username
        print(receive_message(s))  # "Please enter your username:"
        username = input()
        send_message(s, username)
        data = receive_message(s)

        if data is None:
            print("Connection closed by server.")
            return

        if data == "No room in lobby":
            print("No room in the lobby")
            return

        print(data)
        game = GameClient()

        while True:
            user_input = input("With how many point do you want to attack?: ")
            payload = str({"attack": user_input})
            send_message(s, payload)
            data = receive_message(s)
            if data is None:
                print("Connection closed by server.")
                break
            print(f"Received: {data}")


if __name__ == "__main__":
    main()
