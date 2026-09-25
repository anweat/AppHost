'use client';
import {useState} from 'react';
export default function Page() { const [count, setCount] = useState(0); return <main><h1>G2 Next fixture version one</h1><button onClick={() => setCount(count+1)}>Count {count}</button></main>; }
