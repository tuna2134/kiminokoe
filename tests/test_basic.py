import asyncio
import os

import pytest
import discord
import kiminokoe
from dotenv import load_dotenv


@pytest.mark.asyncio
async def test_basic():
    load_dotenv()
    client = discord.Client(intents=discord.Intents.default())

    @client.event
    async def on_ready():
        print("Hi")
        channel = client.get_channel(1054313806718844999)
        vc = await channel.connect(cls=kiminokoe.VoiceClient)
        await asyncio.sleep(2)
        with open("result.wav", "rb") as f:
            vc.play(f.read())
    
    await client.start(os.getenv("DISCORD_TOKEN"))