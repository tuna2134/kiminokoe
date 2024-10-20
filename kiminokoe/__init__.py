import discord

from .kiminokoe import VoiceConnector

import asyncio


class VoiceClient(discord.VoiceProtocol):
    def __init__(self, client, channel):
        super().__init__(client, channel)
        self.channel = channel
        self.client = client
        self._connector = VoiceConnector(channel.guild.id, client.user.id)
        self._wait_voice_server = asyncio.Event()
        self._wait_voice_state = asyncio.Event()

    async def vc_connect(self):
        await self.channel.guild.change_voice_state(channel=self.channel)
    
    async def connect(self, *args, **kwargs):
        await self.vc_connect()
        await self._wait_voice_server.wait()
        await self._wait_voice_state.wait()
        self._connection = await self._connector.connect()
        self._connection.run()

    async def on_voice_server_update(self, data):
        self._connector.token = data.get("token")
        self._connector.endpoint = data.get("endpoint").replace(":443", "")
        self._wait_voice_server.set()
    
    async def on_voice_state_update(self, data):
        self._connector.session_id = data.get("session_id")
        self._wait_voice_state.set()

    def play(self, source):
        self._connection.play(source)